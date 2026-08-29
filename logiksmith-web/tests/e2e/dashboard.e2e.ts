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
});
