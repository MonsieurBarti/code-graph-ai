import { test, expect } from '@playwright/test';

/**
 * Chat Panel E2E tests — RAG-09 phase gate.
 *
 * Prerequisites:
 *   - `cargo run --features web -- serve` running on :7777
 *   - Graph indexed (project with at least one file)
 *
 * Most tests are pure UI tests that work without a backend LLM configured.
 * The citation navigation test requires a backend with an indexed project and
 * configured LLM provider — it is marked with a NOTE in the test body.
 *
 * Run: npx playwright test tests/e2e/chat.spec.ts
 */

test.describe('Chat Panel', () => {
  async function enterGraphView(page: import('@playwright/test').Page) {
    await page.goto('/');
    const entryButton = page
      .locator('button, a')
      .filter({ hasText: /view graph|open graph|enter|explore/i })
      .first();
    await entryButton.click();
    // Wait for canvas to confirm we're in graph view
    await page.locator('canvas').first().waitFor({ state: 'visible', timeout: 15_000 });
  }

  test('chat icon is visible in sidebar rail', async ({ page }) => {
    await enterGraphView(page);
    const chatBtn = page.locator('.icon-rail button[title="Chat with AI"]');
    await expect(chatBtn).toBeVisible();
  });

  test('clicking chat icon opens the chat panel', async ({ page }) => {
    await enterGraphView(page);
    const chatBtn = page.locator('.icon-rail button[title="Chat with AI"]');
    await chatBtn.click();
    const chatPanel = page.locator('.chat-overlay');
    await expect(chatPanel).toBeVisible();
  });

  test('chat panel contains text input', async ({ page }) => {
    await enterGraphView(page);
    await page.locator('.icon-rail button[title="Chat with AI"]').click();
    const input = page.locator('.chat-overlay textarea, .chat-overlay input[type="text"]');
    await expect(input).toBeVisible();
  });

  test('chat panel shows empty state prompt when no messages', async ({ page }) => {
    await enterGraphView(page);
    await page.locator('.icon-rail button[title="Chat with AI"]').click();
    // Should show the empty state with helper text
    const emptyTitle = page.locator('.chat-overlay .empty-title');
    await expect(emptyTitle).toBeVisible();
    await expect(emptyTitle).toContainText('Ask about your codebase');
  });

  test('chat panel collapses to 48px icon strip', async ({ page }) => {
    await enterGraphView(page);
    await page.locator('.icon-rail button[title="Chat with AI"]').click();

    // Wait for panel to open
    await expect(page.locator('.chat-overlay')).toBeVisible();

    // Click the collapse button in the panel header (title contains "Collapse")
    const collapseBtn = page.locator('.chat-overlay button[title*="Collapse"], .chat-overlay button[aria-label*="Collapse"]').first();
    await collapseBtn.click();

    // The collapsed element should be visible
    const collapsed = page.locator('.chat-collapsed');
    await expect(collapsed).toBeVisible();

    // Verify width is 48px
    const box = await collapsed.boundingBox();
    expect(box?.width).toBe(48);
  });

  test('clicking icon strip re-expands the chat panel', async ({ page }) => {
    await enterGraphView(page);
    await page.locator('.icon-rail button[title="Chat with AI"]').click();

    // Collapse it
    const collapseBtn = page.locator('.chat-overlay button[title*="Collapse"], .chat-overlay button[aria-label*="Collapse"]').first();
    await collapseBtn.click();
    await expect(page.locator('.chat-collapsed')).toBeVisible();

    // Click the chat icon in the collapsed strip to expand
    const expandBtn = page.locator('.chat-collapsed button[aria-label="Expand chat"]');
    await expandBtn.click();

    // Should be expanded again — chat panel should be wider than 48px
    const panel = page.locator('.chat-overlay:not(.chat-collapsed)');
    await expect(panel).toBeVisible();
  });

  test('chat panel closes on clicking icon in rail again', async ({ page }) => {
    await enterGraphView(page);
    const chatBtn = page.locator('.icon-rail button[title="Chat with AI"]');

    // Open
    await chatBtn.click();
    await expect(page.locator('.chat-overlay')).toBeVisible();

    // Close by clicking icon again
    await chatBtn.click();
    await expect(page.locator('.chat-overlay')).not.toBeVisible();
  });

  test('chat panel has provider selector', async ({ page }) => {
    await enterGraphView(page);
    await page.locator('.icon-rail button[title="Chat with AI"]').click();
    const providerBtn = page.locator('.chat-overlay .provider-selector');
    await expect(providerBtn).toBeVisible();
  });

  test('citation click navigates graph and opens CodePanel', async ({ page }) => {
    // NOTE: This test requires a running backend with an indexed project and
    // a configured LLM provider (ANTHROPIC_API_KEY or Ollama running).
    // Without a backend, this test will fail on the message send step.
    //
    // The test verifies the full citation flow:
    // 1. Send a chat message
    // 2. Wait for response with citations
    // 3. Click a citation link
    // 4. Verify CodePanel opens at the referenced file

    await enterGraphView(page);
    await page.locator('.icon-rail button[title="Chat with AI"]').click();

    // Type and send a message
    const input = page.locator('.chat-overlay textarea, .chat-overlay input[type="text"]');
    await input.fill('where is the main function');
    await input.press('Enter');

    // Wait for citation link to appear (requires LLM response with citations)
    const citation = page.locator('.chat-overlay .citation-link').first();
    await citation.waitFor({ state: 'visible', timeout: 30_000 });

    // Click the citation
    await citation.click();

    // Verify CodePanel opened (left overlay appears with a file)
    const codePanel = page.locator('.code-overlay');
    await expect(codePanel).toBeVisible({ timeout: 5_000 });
  });
});
