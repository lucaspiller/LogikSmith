import { expect, test } from '@playwright/test';

test.describe('dashboard golden paths', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Automation dashboard' })).toBeVisible();
  });

  test('renders the live block, site time, and schedule data without decoder errors', async ({ page }) => {
    await expect(page.getByRole('button', { name: /scheduled_light_test/ })).toBeVisible();
    await expect(page.getByText('KNX connected')).toBeVisible();
    await expect(page.getByText('Europe/Vilnius')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Schedule morning_on detail' })).toBeVisible();
    await expect(page.locator('body')).not.toContainText(/Malformed (dashboard|automation) data/);
  });

  test('loads occurrence details for a selected schedule', async ({ page }) => {
    await page.getByRole('button', { name: 'Schedule morning_on detail' }).click();

    await expect(page.getByRole('heading', { name: 'Next occurrences' })).toBeVisible();
    const occurrences = page.getByRole('heading', { name: 'Next occurrences' }).locator('xpath=following-sibling::div[1]');
    await expect(occurrences.getByRole('cell', { name: '2026-08-29 05:30:00' })).toBeVisible();
    await expect(occurrences.getByRole('cell', { name: 'Saturday' })).toBeVisible();
    await expect(page.locator('[role="alert"]')).not.toContainText(/Malformed/);
  });

  test('runs a schedule simulation and shows proposed effects', async ({ page }) => {
    await page.getByRole('radio', { name: 'schedule' }).check();
    await page.getByLabel('Simulation schedule').selectOption('morning_on');
    const occurrence = page.getByLabel('Simulation occurrence');
    await expect(occurrence.locator('option')).toHaveCount(3);
    await occurrence.selectOption('1756500000000');

    await page.getByRole('button', { name: 'Run simulation' }).click();

    const result = page.getByRole('article', { name: 'Simulation result' });
    await expect(result).toBeVisible();
    await expect(result.getByRole('heading', { name: 'Simulation succeeded' })).toBeVisible();
    await expect(result).toContainText('scheduled_light');
    await expect(result).toContainText('No KNX write sent.');
  });

  test('renders signal state, producer-disabled status, and the chained causal path', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Signals' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Signal house_occupied detail' })).toBeVisible();
    await expect(page.getByText('producer_disabled')).toBeVisible();
    await page.getByRole('button', { name: 'Signal house_occupied detail' }).click();
    await expect(page.getByRole('article', { name: 'Signal house_occupied detail' })).toContainText('occupancy_source.occupied');
    await expect(page.getByRole('article', { name: 'Signal house_occupied detail' })).toContainText('lighting_policy.occupied');

    await page.getByRole('region', { name: 'Logic blocks' }).getByRole('button', { name: /lighting_policy/ }).click();
    await page.getByRole('tab', { name: 'Inspect' }).click();
    await expect(page.locator('.inspect-view td').filter({ hasText: 'SIGNAL' }).first()).toBeVisible();
    await page.locator('.execution-history tbody tr').first().click();
    const execution = page.locator('article.execution-detail');
    await expect(execution).toContainText('Execution 102');
    await expect(execution).toContainText('occupancy_source / 101');
    await expect(execution).toContainText('lighting_allowed');
  });

  test('shows producer simulation signal effects as eligible-only proposals', async ({ page }) => {
    await page.getByRole('region', { name: 'Logic blocks' }).getByRole('button', { name: /occupancy_source/ }).click();
    await page.getByRole('tab', { name: 'Test' }).click();
    await page.getByRole('radio', { name: 'input' }).check();
    await page.getByLabel('Simulation current trigger value').selectOption('true');
    await page.getByRole('button', { name: 'Run simulation' }).click();

    const capture = page.locator('section[aria-label="Simulation capture"]');
    await expect(capture).toContainText('Proposed signal effects');
    await expect(capture).toContainText('house_occupied');
    await expect(capture).toContainText('Eligible consumers (not executed)');
    await expect(capture).toContainText('This draft does not propagate or execute them');
  });

  test('renders HTTP poll and webhook health, values, and consumers', async ({ page }) => {
    const external = page.getByRole('region', { name: 'External inputs' });
    await expect(external.getByRole('heading', { name: 'External inputs' })).toBeVisible();

    const poll = external.getByRole('article', { name: 'HTTP poll berlin_today_forecast' });
    await expect(poll).toContainText('healthy');
    await expect(poll).toContainText('Next attempt');
    await expect(poll).toContainText('https://api.open-meteo.com/v1/forecast');
    await expect(poll).toContainText('today_temperature_max');
    await expect(poll).toContainText('9.001');
    await expect(poll).toContainText('/daily/temperature_2m_max/0');
    await expect(poll).toContainText('scheduled_light_test.today_temperature_max');

    const webhook = external.getByRole('article', { name: 'Webhook external_override' });
    await expect(webhook).toContainText('healthy');
    await expect(webhook).toContainText('/api/webhooks/external_override');
    await expect(webhook).toContainText('bearer token configured');
    await expect(webhook).toContainText('3 / 1');
    await expect(webhook).toContainText('scheduled_light_test.external_override');
  });

  test('shows external bindings on the selected block', async ({ page }) => {
    await page.getByRole('button', { name: /scheduled_light_test/ }).click();
    await page.getByRole('tab', { name: 'Inspect' }).click();
    const block = page.getByRole('region', { name: 'Selected block' });
    await expect(block).toContainText('today_temperature_max');
    await expect(block).toContainText('http');
    await expect(block).toContainText('external_override');
    await expect(block).toContainText('webhook');
    await expect(block).toContainText('21.75');
  });

  test('toggles the Lua editor between standard and Vim modes', async ({ page }) => {
    const editor = page.getByRole('region', { name: 'Selected block' }).getByRole('region', { name: 'Author view' });
    const enableVim = editor.getByRole('button', { name: 'Enable Vim mode' });
    await expect(enableVim).toBeVisible();
    await expect(enableVim).toHaveAttribute('aria-pressed', 'false');

    await enableVim.click();
    const useStandard = editor.getByRole('button', { name: 'Use standard mode' });
    await expect(useStandard).toHaveAttribute('aria-pressed', 'true');
    await expect(editor.getByText('Editing mode: Vim')).toBeVisible();
    await expect(editor.locator('.cm-scroller')).toHaveClass(/cm-vimMode/);

    await editor.locator('.cm-content .cm-line').first().click({ position: { x: 1, y: 1 } });
    await page.keyboard.press('i');
    await page.keyboard.insertText('XY');
    await expect(editor.locator('.cm-content')).toHaveText(/^XYfunction/);
    await page.keyboard.press('Escape');

    await useStandard.click();
    await expect(editor.getByRole('button', { name: 'Enable Vim mode' })).toHaveAttribute('aria-pressed', 'false');
    await expect(editor.getByText('Editing mode: Standard')).toBeVisible();
    await expect(editor.locator('.cm-scroller')).not.toHaveClass(/cm-vimMode/);
  });

  test('keeps typed Lua edits in place while the draft state updates', async ({ page }) => {
    const editor = page.getByRole('region', { name: 'Selected block' }).getByRole('region', { name: 'Author view' });
    const content = editor.locator('.cm-content');
    await content.locator('.cm-line').first().click({ position: { x: 1, y: 1 } });
    await page.keyboard.insertText('XY');
    await expect(content).toHaveText(/^XYfunction/);
  });

  test('shows HTTP provenance for an external execution', async ({ page }) => {
    await page.getByRole('button', { name: /scheduled_light_test/ }).click();
    await page.getByRole('tab', { name: 'Inspect' }).click();
    const row = page.locator('.execution-history tbody tr').filter({ hasText: 'input:today_temperature_max' });
    await expect(row).toHaveCount(1);
    await row.click();

    const origin = page.getByRole('region', { name: 'Selected execution origin' });
    await expect(origin).toContainText('HTTP poll berlin_today_forecast / today_temperature_max');
  });
});
