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

    await page.getByRole('button', { name: /lighting_policy/ }).click();
    await expect(page.getByText('signal', { exact: true }).first()).toBeVisible();
    await page.locator('.execution-history tbody tr').first().click();
    await expect(page.getByRole('article', { name: /Signal execution details for 102/ })).toContainText('101');
    await expect(page.getByRole('article', { name: /Signal execution details for 102/ })).toContainText('lighting_allowed');
  });

  test('shows producer simulation signal effects as eligible-only proposals', async ({ page }) => {
    await page.getByRole('button', { name: /occupancy_source/ }).click();
    await page.getByRole('radio', { name: 'input' }).check();
    await page.getByLabel('Simulation current trigger value').selectOption('true');
    await page.getByRole('button', { name: 'Run simulation' }).click();

    const capture = page.locator('section[aria-label="Simulation capture"]');
    await expect(capture).toContainText('Proposed signal effects');
    await expect(capture).toContainText('house_occupied');
    await expect(capture).toContainText('Eligible consumers (not executed)');
    await expect(capture).toContainText('does not propagate or execute them');
  });
});
