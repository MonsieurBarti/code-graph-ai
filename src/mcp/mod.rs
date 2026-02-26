mod params;
mod server;

use std::path::PathBuf;

use rmcp::transport::stdio;

pub async fn run(project_root: PathBuf, watch: bool) -> anyhow::Result<()> {
    let service = server::CodeGraphServer::new(project_root, watch);
    let server = rmcp::serve_server(service, stdio()).await?;
    server.waiting().await?;
    Ok(())
}
