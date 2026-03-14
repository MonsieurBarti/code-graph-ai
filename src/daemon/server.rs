use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{RwLock, watch};

use crate::daemon::pid;
use crate::daemon::protocol::{DaemonRequest, DaemonResponse, PROTOCOL_VERSION};
use crate::graph::CodeGraph;

/// Maximum allowed request size in bytes (1 MB).
const MAX_REQUEST_BYTES: usize = 1_048_576;

/// Run the daemon server: build graph, watch for changes, serve queries over Unix socket.
///
/// This function does not return under normal operation. It runs until a Shutdown
/// request, SIGTERM, or SIGINT is received.
pub async fn run_daemon(project_root: PathBuf) -> Result<()> {
    eprintln!("[daemon] starting for project: {}", project_root.display());

    // 1. Build initial graph.
    let graph = tokio::task::spawn_blocking({
        let root = project_root.clone();
        move || crate::build_graph(&root, false)
    })
    .await
    .context("build_graph task panicked")?
    .context("failed to build initial graph")?;

    eprintln!(
        "[daemon] indexed {} files, {} symbols",
        graph.file_count(),
        graph.symbol_count()
    );

    let graph = Arc::new(RwLock::new(graph));

    // 2. Write PID file.
    pid::write_pid_file(&project_root)?;

    // 3. Bind Unix socket (remove stale socket first).
    let sock_path = pid::socket_path(&project_root);
    pid::remove_socket_file(&project_root)?;

    // Ensure parent directory exists.
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("failed to bind socket at {}", sock_path.display()))?;

    // Set socket file permissions to 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&sock_path, perms)
            .with_context(|| format!("failed to set permissions on {}", sock_path.display()))?;
    }

    eprintln!("[daemon] listening on {}", sock_path.display());

    // 4. Shutdown signal channel.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 5. Spawn watcher task.
    let watcher_handle = spawn_watcher(
        project_root.clone(),
        Arc::clone(&graph),
        shutdown_rx.clone(),
    );

    // 6. Spawn signal handler.
    let signal_shutdown_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        eprintln!("[daemon] signal received, shutting down...");
        let _ = signal_shutdown_tx.send(true);
    });

    // 7. Accept connections until shutdown.
    let accept_result = accept_loop(
        listener,
        Arc::clone(&graph),
        project_root.clone(),
        shutdown_tx.clone(),
        shutdown_rx.clone(),
    )
    .await;

    // 8. Graceful shutdown: save cache, remove PID and socket files.
    eprintln!("[daemon] shutting down...");

    // Wait briefly for watcher to stop.
    let _ = tokio::time::timeout(Duration::from_secs(2), watcher_handle).await;

    // Save cache.
    {
        let g = graph.read().await;
        if let Err(e) = crate::cache::save_cache(&project_root, &g) {
            eprintln!("[daemon] failed to save cache on shutdown: {}", e);
        } else {
            eprintln!("[daemon] cache saved");
        }
    }

    // Remove PID and socket files.
    let _ = pid::remove_pid_file(&project_root);
    let _ = pid::remove_socket_file(&project_root);

    eprintln!("[daemon] stopped");
    accept_result
}

/// Accept connections on the Unix socket until shutdown is signaled.
async fn accept_loop(
    listener: UnixListener,
    graph: Arc<RwLock<CodeGraph>>,
    project_root: PathBuf,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _addr)) => {
                        let graph = Arc::clone(&graph);
                        let root = project_root.clone();
                        let tx = shutdown_tx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, graph, root, tx).await {
                                eprintln!("[daemon] connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[daemon] accept error: {}", e);
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Handle a single client connection: read one JSON line, dispatch, write one JSON line.
async fn handle_connection(
    stream: tokio::net::UnixStream,
    graph: Arc<RwLock<CodeGraph>>,
    project_root: PathBuf,
    shutdown_tx: watch::Sender<bool>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    // Read one line (up to MAX_REQUEST_BYTES).
    let mut total_read = 0usize;
    loop {
        let bytes_read = buf_reader
            .read_line(&mut line)
            .await
            .context("failed to read from socket")?;
        if bytes_read == 0 {
            // EOF before newline — client disconnected.
            return Ok(());
        }
        total_read += bytes_read;
        if total_read > MAX_REQUEST_BYTES {
            let response = DaemonResponse::error("request too large (exceeds 1 MB limit)");
            let json = serde_json::to_string(&response)?;
            writer.write_all(json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.shutdown().await?;
            return Ok(());
        }
        if line.ends_with('\n') {
            break;
        }
    }

    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }

    // Validate JSON structure before deserializing into the typed request.
    let json_value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            let response = DaemonResponse::error(format!("invalid JSON: {}", e));
            let json = serde_json::to_string(&response)?;
            writer.write_all(json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.shutdown().await?;
            return Ok(());
        }
    };

    let request: DaemonRequest = match serde_json::from_value(json_value) {
        Ok(r) => r,
        Err(e) => {
            let response = DaemonResponse::error(format!("invalid request: {}", e));
            let json = serde_json::to_string(&response)?;
            writer.write_all(json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.shutdown().await?;
            return Ok(());
        }
    };

    // Handle Shutdown specially — it triggers daemon-wide shutdown.
    if matches!(request, DaemonRequest::Shutdown) {
        let response = DaemonResponse::success(serde_json::json!({"message": "shutting down"}));
        let json = serde_json::to_string(&response)?;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.shutdown().await?;
        let _ = shutdown_tx.send(true);
        return Ok(());
    }

    // Dispatch the query.
    let response = {
        let g = graph.read().await;
        dispatch_query(&request, &g, &project_root)
    };

    let json = serde_json::to_string(&response)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;

    Ok(())
}

/// Spawn the file watcher task that runs in a blocking thread and forwards
/// events to the graph via incremental updates.
fn spawn_watcher(
    project_root: PathBuf,
    graph: Arc<RwLock<CodeGraph>>,
    shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Start the watcher in a blocking context (uses std mpsc).
        let watcher_result = {
            let root = project_root.clone();
            tokio::task::spawn_blocking(move || crate::watcher::start_watcher(&root)).await
        };

        let (_handle, rx) = match watcher_result {
            Ok(Ok((handle, rx))) => (handle, rx),
            Ok(Err(e)) => {
                eprintln!("[daemon] failed to start watcher: {}", e);
                return;
            }
            Err(e) => {
                eprintln!("[daemon] watcher task panicked: {}", e);
                return;
            }
        };

        eprintln!("[daemon] file watcher started");

        // Relay std mpsc events to a tokio mpsc channel using a dedicated
        // blocking thread, then process events asynchronously.
        run_watcher_relay(rx, graph, project_root, shutdown_rx).await;
    })
}

/// Relay events from the std mpsc Receiver to incremental graph updates,
/// batching cache saves.
async fn run_watcher_relay(
    rx: std::sync::mpsc::Receiver<crate::watcher::event::WatchEvent>,
    graph: Arc<RwLock<CodeGraph>>,
    project_root: PathBuf,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    // Bridge: spawn a blocking task that reads from the std receiver
    // and sends to a tokio mpsc channel.
    let (relay_tx, mut relay_rx) =
        tokio::sync::mpsc::channel::<crate::watcher::event::WatchEvent>(256);

    let bridge = tokio::task::spawn_blocking(move || {
        while let Ok(event) = rx.recv() {
            if relay_tx.blocking_send(event).is_err() {
                break; // receiver dropped
            }
        }
    });

    // Timer for batched cache saves — save at most once per second.
    let mut dirty = false;
    let mut save_interval = tokio::time::interval(Duration::from_secs(1));
    save_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip the first immediate tick.
    save_interval.tick().await;

    loop {
        tokio::select! {
            event = relay_rx.recv() => {
                match event {
                    Some(ev) => {
                        handle_watcher_event(&ev, &graph, &project_root).await;
                        dirty = true;
                    }
                    None => break, // bridge thread finished
                }
            }
            _ = save_interval.tick(), if dirty => {
                let g = graph.read().await;
                if let Err(e) = crate::cache::save_cache(&project_root, &g) {
                    eprintln!("[daemon] cache save error: {}", e);
                }
                dirty = false;
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    // Final cache save if dirty.
    if dirty {
        let g = graph.read().await;
        let _ = crate::cache::save_cache(&project_root, &g);
    }

    // Clean up bridge thread.
    drop(relay_rx);
    let _ = bridge.await;
}

/// Process a single watcher event, updating the graph.
async fn handle_watcher_event(
    event: &crate::watcher::event::WatchEvent,
    graph: &Arc<RwLock<CodeGraph>>,
    project_root: &Path,
) {
    use crate::watcher::event::WatchEvent;

    match event {
        WatchEvent::Modified(p) => {
            let start = std::time::Instant::now();
            {
                let mut g = graph.write().await;
                crate::watcher::incremental::handle_file_event(&mut g, event, project_root);
            }
            let elapsed = start.elapsed();
            eprintln!(
                "[daemon] incremental: {} ({:.1}ms)",
                p.strip_prefix(project_root).unwrap_or(p).display(),
                elapsed.as_secs_f64() * 1000.0,
            );
        }
        WatchEvent::Deleted(p) => {
            let mut g = graph.write().await;
            crate::watcher::incremental::handle_file_event(&mut g, event, project_root);
            eprintln!(
                "[daemon] deleted: {} ({} files, {} symbols)",
                p.strip_prefix(project_root).unwrap_or(p).display(),
                g.file_count(),
                g.symbol_count(),
            );
        }
        WatchEvent::ConfigChanged => {
            eprintln!("[daemon] config changed -- full re-index...");
            let start = std::time::Instant::now();
            let root = project_root.to_path_buf();
            match tokio::task::spawn_blocking(move || crate::build_graph(&root, false)).await {
                Ok(Ok(new_graph)) => {
                    let mut g = graph.write().await;
                    *g = new_graph;
                    let elapsed = start.elapsed();
                    eprintln!(
                        "[daemon] re-indexed in {:.1}ms ({} files, {} symbols)",
                        elapsed.as_secs_f64() * 1000.0,
                        g.file_count(),
                        g.symbol_count(),
                    );
                }
                Ok(Err(e)) => {
                    eprintln!("[daemon] full re-index failed: {}", e);
                }
                Err(e) => {
                    eprintln!("[daemon] re-index task panicked: {}", e);
                }
            }
        }
        WatchEvent::CrateRootChanged(p) => {
            let filename = p.file_name().unwrap_or_default().to_string_lossy();
            eprintln!("[daemon] full re-index: {} changed", filename);
            let start = std::time::Instant::now();
            let root = project_root.to_path_buf();
            match tokio::task::spawn_blocking(move || crate::build_graph(&root, false)).await {
                Ok(Ok(new_graph)) => {
                    let mut g = graph.write().await;
                    *g = new_graph;
                    let elapsed = start.elapsed();
                    eprintln!(
                        "[daemon] re-indexed in {:.1}ms ({} files, {} symbols)",
                        elapsed.as_secs_f64() * 1000.0,
                        g.file_count(),
                        g.symbol_count(),
                    );
                }
                Ok(Err(e)) => {
                    eprintln!("[daemon] full re-index failed: {}", e);
                }
                Err(e) => {
                    eprintln!("[daemon] re-index task panicked: {}", e);
                }
            }
        }
    }
}

/// Wait for SIGTERM or SIGINT.
async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
        let mut sigint =
            signal(SignalKind::interrupt()).expect("failed to register SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        // Fallback for non-Unix: wait for Ctrl+C.
        let _ = tokio::signal::ctrl_c().await;
    }
}

// ---------------------------------------------------------------------------
// Query dispatch
// ---------------------------------------------------------------------------

/// Dispatch a `DaemonRequest` to the appropriate query function and return a
/// `DaemonResponse`.
///
/// This is the central query router. It mirrors the CLI command dispatch in
/// `main.rs` but operates on a shared `&CodeGraph` reference.
fn dispatch_query(
    request: &DaemonRequest,
    graph: &CodeGraph,
    project_root: &Path,
) -> DaemonResponse {
    match request {
        DaemonRequest::Ping => DaemonResponse::success(serde_json::json!({
            "daemon": "code-graph",
            "version": PROTOCOL_VERSION,
            "pid": std::process::id(),
        })),

        DaemonRequest::Shutdown => {
            // Handled before dispatch_query is called; included for exhaustiveness.
            DaemonResponse::success(serde_json::json!({"message": "shutting down"}))
        }

        DaemonRequest::Find {
            symbol,
            case_insensitive,
            kind,
            file,
            language,
        } => dispatch_find(
            graph,
            project_root,
            symbol,
            *case_insensitive,
            kind,
            file.as_deref(),
            language.as_deref(),
        ),

        DaemonRequest::Refs {
            symbol,
            case_insensitive,
            kind: _,
            file: _,
            language,
        } => dispatch_refs(
            graph,
            project_root,
            symbol,
            *case_insensitive,
            language.as_deref(),
        ),

        DaemonRequest::Impact {
            symbol,
            case_insensitive,
            tree: _,
            language,
        } => dispatch_impact(
            graph,
            project_root,
            symbol,
            *case_insensitive,
            language.as_deref(),
        ),

        DaemonRequest::Context {
            symbol,
            case_insensitive,
            language,
        } => dispatch_context(
            graph,
            project_root,
            symbol,
            *case_insensitive,
            language.as_deref(),
        ),

        DaemonRequest::Stats { language } => dispatch_stats(graph, language.as_deref()),

        DaemonRequest::Circular { language } => {
            dispatch_circular(graph, project_root, language.as_deref())
        }

        DaemonRequest::DeadCode { scope } => {
            dispatch_dead_code(graph, project_root, scope.as_deref())
        }

        DaemonRequest::Export {
            format,
            granularity,
            stdout: _,
            root,
            symbol,
            depth,
            exclude,
        } => dispatch_export(
            graph,
            project_root,
            format,
            granularity,
            root.as_deref(),
            symbol.as_deref(),
            *depth,
            exclude,
        ),

        DaemonRequest::Structure { path, depth } => {
            dispatch_structure(graph, project_root, path.as_deref(), *depth)
        }

        DaemonRequest::FileSummary { file } => dispatch_file_summary(graph, project_root, file),

        DaemonRequest::Imports { file } => dispatch_imports(graph, project_root, file),

        DaemonRequest::Diff { from, to } => dispatch_diff(graph, project_root, from, to.as_deref()),

        DaemonRequest::DiffImpact { base_ref } => {
            dispatch_diff_impact(graph, project_root, base_ref)
        }

        DaemonRequest::Decorators {
            pattern,
            language,
            framework,
        } => dispatch_decorators(graph, pattern, language.as_deref(), framework.as_deref()),

        DaemonRequest::Clusters { scope } => {
            dispatch_clusters(graph, project_root, scope.as_deref())
        }

        DaemonRequest::Flow {
            entry,
            target,
            max_paths,
            max_depth,
        } => dispatch_flow(graph, entry, target, *max_paths, *max_depth),

        DaemonRequest::Rename { symbol, new_name } => {
            dispatch_rename(graph, project_root, symbol, new_name)
        }

        DaemonRequest::SnapshotCreate { name } => {
            dispatch_snapshot_create(graph, project_root, name)
        }

        DaemonRequest::SnapshotList => dispatch_snapshot_list(project_root),

        DaemonRequest::SnapshotDelete { name } => dispatch_snapshot_delete(project_root, name),
    }
}

// ---------------------------------------------------------------------------
// Individual dispatch helpers
// ---------------------------------------------------------------------------

fn dispatch_find(
    graph: &CodeGraph,
    project_root: &Path,
    symbol: &str,
    case_insensitive: bool,
    kind_filter: &[String],
    file_filter: Option<&Path>,
    language: Option<&str>,
) -> DaemonResponse {
    let language_filter = match parse_lang(language) {
        Ok(f) => f,
        Err(e) => return DaemonResponse::error(e),
    };

    match crate::query::find::find_symbol(
        graph,
        symbol,
        case_insensitive,
        kind_filter,
        file_filter,
        project_root,
        language_filter,
    ) {
        Ok(results) => {
            let data: Vec<serde_json::Value> = results
                .iter()
                .map(|r| find_result_to_json(r, project_root))
                .collect();
            DaemonResponse::success(serde_json::json!(data))
        }
        Err(e) => DaemonResponse::error(format!("{}", e)),
    }
}

fn dispatch_refs(
    graph: &CodeGraph,
    project_root: &Path,
    symbol: &str,
    case_insensitive: bool,
    language: Option<&str>,
) -> DaemonResponse {
    let language_filter = match parse_lang(language) {
        Ok(f) => f,
        Err(e) => return DaemonResponse::error(e),
    };

    let matches = match crate::query::find::match_symbols(graph, symbol, case_insensitive) {
        Ok(m) => m,
        Err(e) => return DaemonResponse::error(format!("{}", e)),
    };

    if matches.is_empty() {
        return DaemonResponse::error(format!("no symbols matching '{}' found", symbol));
    }

    let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
        .iter()
        .flat_map(|(_, indices)| indices.iter().copied())
        .collect();

    let mut results = crate::query::refs::find_refs(graph, symbol, &all_indices, project_root);

    if let Some(lang) = language_filter {
        results.retain(|r| file_language_matches(&r.file_path, lang));
    }

    let data: Vec<serde_json::Value> = results
        .iter()
        .map(|r| ref_result_to_json(r, project_root))
        .collect();
    DaemonResponse::success(serde_json::json!(data))
}

fn dispatch_impact(
    graph: &CodeGraph,
    project_root: &Path,
    symbol: &str,
    case_insensitive: bool,
    language: Option<&str>,
) -> DaemonResponse {
    let language_filter = match parse_lang(language) {
        Ok(f) => f,
        Err(e) => return DaemonResponse::error(e),
    };

    let matches = match crate::query::find::match_symbols(graph, symbol, case_insensitive) {
        Ok(m) => m,
        Err(e) => return DaemonResponse::error(format!("{}", e)),
    };

    if matches.is_empty() {
        return DaemonResponse::error(format!("no symbols matching '{}' found", symbol));
    }

    let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
        .iter()
        .flat_map(|(_, indices)| indices.iter().copied())
        .collect();

    let mut results = crate::query::impact::blast_radius(graph, &all_indices, project_root);

    if let Some(lang) = language_filter {
        results.retain(|r| file_language_matches(&r.file_path, lang));
    }

    match serde_json::to_value(&results) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_context(
    graph: &CodeGraph,
    project_root: &Path,
    symbol: &str,
    case_insensitive: bool,
    language: Option<&str>,
) -> DaemonResponse {
    let language_filter = match parse_lang(language) {
        Ok(f) => f,
        Err(e) => return DaemonResponse::error(e),
    };

    let matches = match crate::query::find::match_symbols(graph, symbol, case_insensitive) {
        Ok(m) => m,
        Err(e) => return DaemonResponse::error(format!("{}", e)),
    };

    if matches.is_empty() {
        return DaemonResponse::error(format!("no symbols matching '{}' found", symbol));
    }

    let mut results: Vec<crate::query::context::SymbolContext> = matches
        .iter()
        .map(|(name, indices)| {
            crate::query::context::symbol_context(graph, name, indices, project_root)
        })
        .collect();

    if let Some(lang) = language_filter {
        for ctx in &mut results {
            ctx.definitions
                .retain(|d| file_language_matches(&d.file_path, lang));
            ctx.references
                .retain(|r| file_language_matches(&r.file_path, lang));
            ctx.callers
                .retain(|c| file_language_matches(&c.file_path, lang));
            ctx.callees
                .retain(|c| file_language_matches(&c.file_path, lang));
        }
        results.retain(|ctx| !ctx.definitions.is_empty());
    }

    let data: Vec<serde_json::Value> = results
        .iter()
        .map(|ctx| context_to_json(ctx, project_root))
        .collect();
    DaemonResponse::success(serde_json::json!(data))
}

fn dispatch_stats(graph: &CodeGraph, language: Option<&str>) -> DaemonResponse {
    let _language_filter = match parse_lang(language) {
        Ok(f) => f,
        Err(e) => return DaemonResponse::error(e),
    };

    let stats = crate::query::stats::project_stats(graph);
    DaemonResponse::success(stats_to_json(&stats))
}

fn dispatch_circular(
    graph: &CodeGraph,
    project_root: &Path,
    language: Option<&str>,
) -> DaemonResponse {
    let language_filter = match parse_lang(language) {
        Ok(f) => f,
        Err(e) => return DaemonResponse::error(e),
    };

    let mut cycles = crate::query::circular::find_circular(graph, project_root);

    if let Some(lang) = language_filter {
        cycles.retain(|c| c.files.iter().all(|f| file_language_matches(f, lang)));
    }

    let data: Vec<serde_json::Value> = cycles
        .iter()
        .map(|c| {
            let files: Vec<String> = c
                .files
                .iter()
                .map(|f| {
                    f.strip_prefix(project_root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            serde_json::json!({ "files": files })
        })
        .collect();
    DaemonResponse::success(serde_json::json!(data))
}

fn dispatch_dead_code(
    graph: &CodeGraph,
    project_root: &Path,
    scope: Option<&Path>,
) -> DaemonResponse {
    let result = crate::query::dead_code::find_dead_code(graph, project_root, scope);
    match serde_json::to_value(&result) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_export(
    graph: &CodeGraph,
    project_root: &Path,
    format: &str,
    granularity: &str,
    root_filter: Option<&Path>,
    symbol_filter: Option<&str>,
    depth: usize,
    exclude: &[String],
) -> DaemonResponse {
    let fmt = match format {
        "dot" => crate::export::model::ExportFormat::Dot,
        "mermaid" => crate::export::model::ExportFormat::Mermaid,
        other => {
            return DaemonResponse::error(format!(
                "unknown export format '{}'. Valid: dot, mermaid",
                other
            ));
        }
    };

    let gran = match granularity {
        "symbol" => crate::export::model::Granularity::Symbol,
        "file" => crate::export::model::Granularity::File,
        "package" => crate::export::model::Granularity::Package,
        other => {
            return DaemonResponse::error(format!(
                "unknown granularity '{}'. Valid: symbol, file, package",
                other
            ));
        }
    };

    let params = crate::export::model::ExportParams {
        format: fmt,
        granularity: gran,
        root_filter: root_filter.map(|p| p.to_path_buf()),
        symbol_filter: symbol_filter.map(|s| s.to_string()),
        depth,
        exclude_patterns: exclude.to_vec(),
        project_root: project_root.to_path_buf(),
        stdout: true,
    };

    match crate::export::export_graph(graph, &params) {
        Ok(result) => DaemonResponse::success(serde_json::json!({
            "content": result.content,
            "node_count": result.node_count,
            "edge_count": result.edge_count,
        })),
        Err(e) => DaemonResponse::error(format!("{}", e)),
    }
}

fn dispatch_structure(
    graph: &CodeGraph,
    project_root: &Path,
    path: Option<&Path>,
    depth: usize,
) -> DaemonResponse {
    let tree = crate::query::structure::file_structure(graph, project_root, path, depth);
    match serde_json::to_value(&tree) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_file_summary(graph: &CodeGraph, project_root: &Path, file: &Path) -> DaemonResponse {
    match crate::query::file_summary::file_summary(graph, project_root, file) {
        Ok(summary) => match serde_json::to_value(&summary) {
            Ok(data) => DaemonResponse::success(data),
            Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
        },
        Err(e) => DaemonResponse::error(e),
    }
}

fn dispatch_imports(graph: &CodeGraph, project_root: &Path, file: &Path) -> DaemonResponse {
    match crate::query::imports::file_imports(graph, project_root, file) {
        Ok(entries) => match serde_json::to_value(&entries) {
            Ok(data) => DaemonResponse::success(data),
            Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
        },
        Err(e) => DaemonResponse::error(e),
    }
}

fn dispatch_diff(
    graph: &CodeGraph,
    project_root: &Path,
    from: &str,
    to: Option<&str>,
) -> DaemonResponse {
    match crate::query::diff::compute_diff(project_root, from, to, graph) {
        Ok(diff) => match serde_json::to_value(&diff) {
            Ok(data) => DaemonResponse::success(data),
            Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
        },
        Err(e) => DaemonResponse::error(e),
    }
}

fn dispatch_diff_impact(graph: &CodeGraph, project_root: &Path, base_ref: &str) -> DaemonResponse {
    // Shell out to git diff --name-only
    let output = match std::process::Command::new("git")
        .args(["diff", "--name-only", base_ref])
        .current_dir(project_root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            return DaemonResponse::error(format!(
                "failed to run git: {}. Ensure git is in PATH.",
                e
            ));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return DaemonResponse::error(format!("git diff failed: {}", stderr));
    }

    let changed_files: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| project_root.join(l))
        .collect();

    if changed_files.is_empty() {
        return DaemonResponse::success(
            serde_json::json!({"message": "no changed files", "results": []}),
        );
    }

    let config = crate::config::CodeGraphConfig::load(project_root);
    let results = crate::query::impact::diff_impact(
        graph,
        &changed_files,
        project_root,
        config.impact.high_threshold,
        config.impact.medium_threshold,
    );

    match serde_json::to_value(&results) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_decorators(
    graph: &CodeGraph,
    pattern: &str,
    language: Option<&str>,
    framework: Option<&str>,
) -> DaemonResponse {
    match crate::query::decorators::find_by_decorator(graph, pattern, language, framework, 100) {
        Ok(results) => match serde_json::to_value(&results) {
            Ok(data) => DaemonResponse::success(data),
            Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
        },
        Err(e) => DaemonResponse::error(format!("{}", e)),
    }
}

fn dispatch_clusters(
    graph: &CodeGraph,
    project_root: &Path,
    scope: Option<&Path>,
) -> DaemonResponse {
    let results = crate::query::clusters::find_clusters(graph, project_root, scope, 100);
    match serde_json::to_value(&results) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_flow(
    graph: &CodeGraph,
    entry: &str,
    target: &str,
    max_paths: usize,
    max_depth: usize,
) -> DaemonResponse {
    let result = crate::query::flow::trace_flow(graph, entry, target, max_paths, max_depth);
    match serde_json::to_value(&result) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_rename(
    graph: &CodeGraph,
    project_root: &Path,
    symbol: &str,
    new_name: &str,
) -> DaemonResponse {
    let items = crate::query::rename::plan_rename(graph, symbol, new_name, project_root);
    match serde_json::to_value(&items) {
        Ok(data) => DaemonResponse::success(data),
        Err(e) => DaemonResponse::error(format!("serialization error: {}", e)),
    }
}

fn dispatch_snapshot_create(graph: &CodeGraph, project_root: &Path, name: &str) -> DaemonResponse {
    match crate::query::diff::create_snapshot(graph, project_root, name) {
        Ok(()) => DaemonResponse::success(
            serde_json::json!({"message": format!("snapshot '{}' created", name)}),
        ),
        Err(e) => DaemonResponse::error(format!("{}", e)),
    }
}

fn dispatch_snapshot_list(project_root: &Path) -> DaemonResponse {
    match crate::query::diff::list_snapshots(project_root) {
        Ok(snapshots) => {
            let data: Vec<serde_json::Value> = snapshots
                .iter()
                .map(|(name, ts)| serde_json::json!({"name": name, "created_at": ts}))
                .collect();
            DaemonResponse::success(serde_json::json!(data))
        }
        Err(e) => DaemonResponse::error(format!("{}", e)),
    }
}

fn dispatch_snapshot_delete(project_root: &Path, name: &str) -> DaemonResponse {
    match crate::query::diff::delete_snapshot(project_root, name) {
        Ok(()) => DaemonResponse::success(
            serde_json::json!({"message": format!("snapshot '{}' deleted", name)}),
        ),
        Err(e) => DaemonResponse::error(format!("{}", e)),
    }
}

// ---------------------------------------------------------------------------
// JSON serialization helpers for types without Serialize derive
// ---------------------------------------------------------------------------

fn find_result_to_json(
    r: &crate::query::find::FindResult,
    project_root: &Path,
) -> serde_json::Value {
    let rel = r
        .file_path
        .strip_prefix(project_root)
        .unwrap_or(&r.file_path);
    serde_json::json!({
        "name": r.symbol_name,
        "kind": crate::query::find::kind_to_str(&r.kind),
        "file": rel.to_string_lossy(),
        "line": r.line,
        "line_end": r.line_end,
        "col": r.col,
        "exported": r.is_exported,
        "default": r.is_default,
    })
}

fn ref_result_to_json(r: &crate::query::refs::RefResult, project_root: &Path) -> serde_json::Value {
    let rel = r
        .file_path
        .strip_prefix(project_root)
        .unwrap_or(&r.file_path);
    serde_json::json!({
        "file": rel.to_string_lossy(),
        "ref_kind": format!("{:?}", r.ref_kind).to_lowercase(),
        "symbol_name": r.symbol_name,
        "line": r.line,
    })
}

fn context_to_json(
    ctx: &crate::query::context::SymbolContext,
    project_root: &Path,
) -> serde_json::Value {
    serde_json::json!({
        "symbol_name": ctx.symbol_name,
        "definitions": ctx.definitions.iter().map(|d| find_result_to_json(d, project_root)).collect::<Vec<_>>(),
        "references": ctx.references.iter().map(|r| ref_result_to_json(r, project_root)).collect::<Vec<_>>(),
        "callers": ctx.callers.iter().map(|c| call_info_to_json(c, project_root)).collect::<Vec<_>>(),
        "callees": ctx.callees.iter().map(|c| call_info_to_json(c, project_root)).collect::<Vec<_>>(),
        "extends": ctx.extends.iter().map(|c| call_info_to_json(c, project_root)).collect::<Vec<_>>(),
        "implements": ctx.implements.iter().map(|c| call_info_to_json(c, project_root)).collect::<Vec<_>>(),
        "extended_by": ctx.extended_by.iter().map(|c| call_info_to_json(c, project_root)).collect::<Vec<_>>(),
        "implemented_by": ctx.implemented_by.iter().map(|c| call_info_to_json(c, project_root)).collect::<Vec<_>>(),
    })
}

fn call_info_to_json(
    c: &crate::query::context::CallInfo,
    project_root: &Path,
) -> serde_json::Value {
    let rel = c
        .file_path
        .strip_prefix(project_root)
        .unwrap_or(&c.file_path);
    serde_json::json!({
        "symbol_name": c.symbol_name,
        "file": rel.to_string_lossy(),
        "line": c.line,
    })
}

fn stats_to_json(stats: &crate::query::stats::ProjectStats) -> serde_json::Value {
    serde_json::json!({
        "file_count": stats.file_count,
        "symbol_count": stats.symbol_count,
        "functions": stats.functions,
        "classes": stats.classes,
        "interfaces": stats.interfaces,
        "type_aliases": stats.type_aliases,
        "enums": stats.enums,
        "variables": stats.variables,
        "components": stats.components,
        "methods": stats.methods,
        "properties": stats.properties,
        "import_edges": stats.import_edges,
        "external_packages": stats.external_packages,
        "unresolved_imports": stats.unresolved_imports,
        "rust_fns": stats.rust_fns,
        "rust_structs": stats.rust_structs,
        "rust_enums": stats.rust_enums,
        "rust_traits": stats.rust_traits,
        "rust_impl_methods": stats.rust_impl_methods,
        "rust_type_aliases": stats.rust_type_aliases,
        "rust_consts": stats.rust_consts,
        "rust_statics": stats.rust_statics,
        "rust_macros": stats.rust_macros,
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Parse a language filter string into a canonical language name.
fn parse_lang(lang: Option<&str>) -> Result<Option<&'static str>, String> {
    match lang {
        None => Ok(None),
        Some(s) => {
            use crate::language::LanguageKind;
            match LanguageKind::from_str_loose(s) {
                Some(LanguageKind::Rust) => Ok(Some("rust")),
                Some(LanguageKind::TypeScript) => Ok(Some("typescript")),
                Some(LanguageKind::JavaScript) => Ok(Some("javascript")),
                Some(LanguageKind::Python) => Ok(Some("python")),
                Some(LanguageKind::Go) => Ok(Some("go")),
                None => Err(format!(
                    "unknown language '{}'. Valid: rust/rs, typescript/ts, javascript/js, python/py, go/golang",
                    s
                )),
            }
        }
    }
}

/// Returns true if the file at `path` belongs to the given language string.
fn file_language_matches(path: &Path, lang: &str) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match lang {
        "rust" => ext == "rs",
        "typescript" => matches!(ext, "ts" | "tsx"),
        "javascript" => matches!(ext, "js" | "jsx"),
        "python" => ext == "py",
        "go" => ext == "go",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_ping_returns_success() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test");
        let response = dispatch_query(&DaemonRequest::Ping, &graph, &root);
        match response {
            DaemonResponse::Success { version, data } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(data["daemon"], "code-graph");
                assert_eq!(data["version"], PROTOCOL_VERSION);
            }
            DaemonResponse::Error { .. } => panic!("expected Success for Ping"),
        }
    }

    #[test]
    fn dispatch_stats_returns_success() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test");
        let response = dispatch_query(&DaemonRequest::Stats { language: None }, &graph, &root);
        match response {
            DaemonResponse::Success { version, data } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(data["file_count"], 0);
                assert_eq!(data["symbol_count"], 0);
            }
            DaemonResponse::Error { .. } => panic!("expected Success for Stats"),
        }
    }

    #[test]
    fn dispatch_find_no_results() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test");
        let response = dispatch_query(
            &DaemonRequest::Find {
                symbol: "NonExistent".into(),
                case_insensitive: false,
                kind: vec![],
                file: None,
                language: None,
            },
            &graph,
            &root,
        );
        match response {
            DaemonResponse::Success { data, .. } => {
                assert!(data.as_array().unwrap().is_empty());
            }
            DaemonResponse::Error { .. } => panic!("expected Success (empty) for Find"),
        }
    }

    #[test]
    fn dispatch_circular_empty_graph() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test");
        let response = dispatch_query(&DaemonRequest::Circular { language: None }, &graph, &root);
        match response {
            DaemonResponse::Success { data, .. } => {
                assert!(data.as_array().unwrap().is_empty());
            }
            DaemonResponse::Error { .. } => panic!("expected Success for Circular"),
        }
    }

    #[test]
    fn dispatch_refs_no_matches() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test");
        let response = dispatch_query(
            &DaemonRequest::Refs {
                symbol: "Nonexistent".into(),
                case_insensitive: false,
                kind: vec![],
                file: None,
                language: None,
            },
            &graph,
            &root,
        );
        // Should be an error because no symbols match.
        match response {
            DaemonResponse::Error { message, .. } => {
                assert!(message.contains("no symbols matching"));
            }
            DaemonResponse::Success { .. } => panic!("expected Error for Refs with no matches"),
        }
    }

    #[test]
    fn dispatch_invalid_language() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test");
        let response = dispatch_query(
            &DaemonRequest::Stats {
                language: Some("invalid_lang".into()),
            },
            &graph,
            &root,
        );
        match response {
            DaemonResponse::Error { message, .. } => {
                assert!(message.contains("unknown language"));
            }
            DaemonResponse::Success { .. } => panic!("expected Error for invalid language"),
        }
    }

    #[test]
    fn parse_lang_valid() {
        assert_eq!(parse_lang(None), Ok(None));
        assert_eq!(parse_lang(Some("rust")), Ok(Some("rust")));
        assert_eq!(parse_lang(Some("rs")), Ok(Some("rust")));
        assert_eq!(parse_lang(Some("typescript")), Ok(Some("typescript")));
        assert_eq!(parse_lang(Some("ts")), Ok(Some("typescript")));
        assert_eq!(parse_lang(Some("python")), Ok(Some("python")));
        assert_eq!(parse_lang(Some("go")), Ok(Some("go")));
    }

    #[test]
    fn parse_lang_invalid() {
        assert!(parse_lang(Some("fortran")).is_err());
    }
}
