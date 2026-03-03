import { test, expect } from '@playwright/test';

/**
 * Smoke test: verifies the web UI loads, shows landing page,
 * and (when a graph is available) enters the graph view.
 *
 * Prerequisite: `cargo run --features web -- serve` must be running on :7777
 */

test.describe('Web UI smoke tests', () => {
  test('landing page loads', async ({ page }) => {
    await page.goto('/');
    // The landing page should render with some heading or button
    await expect(page).toHaveTitle(/code.graph|code graph/i);
  });

  test('graph view entry button is present on landing', async ({ page }) => {
    await page.goto('/');
    // Look for the "View Graph" or "Enter" button on the landing screen
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await expect(entryButton).toBeVisible({ timeout: 10_000 });
  });

  test('graph canvas renders after entering graph view', async ({ page }) => {
    await page.goto('/');
    // Click the entry button to go to graph view
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    // Sigma.js renders into a <canvas> element
    const canvas = page.locator('canvas').first();
    await expect(canvas).toBeVisible({ timeout: 15_000 });
  });

  // Phase 19.1 smoke tests — selection bar, layout indicator, legend interactivity

  test('selection bar is not visible initially (no selection)', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    // Wait for canvas to appear
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    // SelectionBar should not be visible when no node is selected
    await expect(page.locator('.selection-bar')).not.toBeVisible();
  });

  test('status bar shows layout status', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    // Wait for canvas to appear
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    // StatusBar should show "Layout running" or "Ready"
    const statusBar = page.locator('text=Layout running').or(page.locator('text=Ready'));
    await expect(statusBar).toBeVisible({ timeout: 20_000 });
  });

  test('legend edge type rows are interactive and toggle disabled state', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    // Wait for canvas to appear
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    // Click Filters tab in sidebar to access legend
    const filtersBtn = page.locator('button[aria-label="Filters"]');
    await filtersBtn.click();
    // Find edge type rows in legend
    const edgeRow = page.locator('.legend-row-edge').first();
    await expect(edgeRow).toBeVisible();
    // Verify it is initially fully visible (not disabled)
    await expect(edgeRow).not.toHaveClass(/legend-row-disabled/);
    // Click the row to toggle it off
    await edgeRow.click();
    // After clicking, the row should have the disabled class
    await expect(edgeRow).toHaveClass(/legend-row-disabled/);
    // Click again to re-enable
    await edgeRow.click();
    await expect(edgeRow).not.toHaveClass(/legend-row-disabled/);
  });

  test('Escape key does not throw and selection bar remains hidden when no node selected', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    // Wait for canvas
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    // No node selected — press Escape should be a no-op for selection bar
    await page.keyboard.press('Escape');
    // Selection bar should remain hidden
    await expect(page.locator('.selection-bar')).not.toBeVisible();
    // No JavaScript errors should have occurred — verify page is still functional
    await expect(page.locator('canvas').first()).toBeVisible();
  });

  // Phase 19.2 smoke tests — Header bar, StatusBar, sidebar tabs

  test('header bar shows branding and stats', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    await expect(page.locator('text=code-graph')).toBeVisible();
    await expect(page.locator('text=nodes')).toBeVisible();
  });

  test('sidebar has Explorer and Filters tabs', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    await expect(page.locator('button[aria-label="Explorer"]')).toBeVisible();
    await expect(page.locator('button[aria-label="Filters"]')).toBeVisible();
  });

  test('sidebar filters tab contains GranularityToggle and DepthFilter', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    // Open Filters tab
    const filtersBtn = page.locator('button[aria-label="Filters"]');
    await filtersBtn.click();
    // Should show View and Depth filter sections
    await expect(page.locator('text=View')).toBeVisible();
    await expect(page.locator('text=Depth')).toBeVisible();
  });
});

// Phase 19.3 — Folder nodes + selection polish

test.describe('Phase 19.3 — Folder nodes + selection polish', () => {
  test('file graph API contains folder nodes', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });

    const data = await page.evaluate(async () => {
      const res = await fetch('/api/graph?granularity=file');
      return res.json();
    });
    const folderNodes = data.nodes.filter((n: any) => n.attributes.kind === 'folder');
    expect(folderNodes.length).toBeGreaterThan(0);
    expect(folderNodes[0].attributes.color).toBe('#6366f1');
    expect(folderNodes[0].key).toMatch(/^folder:/);
  });

  test('file graph API contains Contains edges', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });

    const data = await page.evaluate(async () => {
      const res = await fetch('/api/graph?granularity=file');
      return res.json();
    });
    const containsEdges = data.edges.filter((e: any) => e.attributes.edgeType === 'Contains');
    expect(containsEdges.length).toBeGreaterThan(0);
    expect(containsEdges[0].attributes.color).toBe('#2d5a3d');
    expect(containsEdges[0].attributes.weight).toBe(0.5);
  });

  test('file graph API has module kind for entry-point files', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });

    const data = await page.evaluate(async () => {
      const res = await fetch('/api/graph?granularity=file');
      return res.json();
    });
    const moduleNodes = data.nodes.filter((n: any) => n.attributes.kind === 'module');
    expect(moduleNodes.length).toBeGreaterThan(0);
    const moduleFilenames = ['mod.rs', 'lib.rs', 'main.rs', 'index.ts', 'index.js', '__init__.py'];
    const hasKnownModuleFile = moduleNodes.some((n: any) =>
      moduleFilenames.some(name => n.attributes.label === name)
    );
    expect(hasKnownModuleFile).toBe(true);
  });

  test('legend shows Folder and Module entries in file view', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    const filtersBtn = page.locator('button[aria-label="Filters"]');
    await filtersBtn.click();
    await expect(page.locator('.legend').locator('text=Folder')).toBeVisible();
    await expect(page.locator('.legend').locator('text=Module')).toBeVisible();
    await expect(page.locator('.legend').locator('text=Node Types')).toBeVisible();
  });

  test('Contains edge toggle works in legend', async ({ page }) => {
    await page.goto('/');
    const entryButton = page.locator('button, a').filter({ hasText: /view graph|open graph|enter|explore/i }).first();
    await entryButton.click();
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
    const filtersBtn = page.locator('button[aria-label="Filters"]');
    await filtersBtn.click();
    const containsRow = page.locator('.legend-row-edge').filter({ hasText: 'Contains' });
    await expect(containsRow).toBeVisible();
    await expect(containsRow).not.toHaveClass(/legend-row-disabled/);
    await containsRow.click();
    await expect(containsRow).toHaveClass(/legend-row-disabled/);
    await containsRow.click();
    await expect(containsRow).not.toHaveClass(/legend-row-disabled/);
  });
});
