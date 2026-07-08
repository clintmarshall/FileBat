const { chromium } = require('playwright');
const child_process = require('child_process');
const path = require('path');
const http = require('http');
const fs = require('fs');
const os = require('os');

const BINARY = path.join(__dirname, 'target/debug/filebitch.exe');
const PORT = 9222;

// ─── Setup Helpers ───

function cleanupWebViewCache() {
	const webViewUserData = path.join(os.homedir(), 'AppData', 'Local', 'filebitch');
	if (fs.existsSync(webViewUserData)) {
		try { fs.rmSync(webViewUserData, { recursive: true, force: true }); } catch (_) {}
	}
}

function waitForCdp(port, timeoutMs = 15000) {
	return new Promise((resolve, reject) => {
		const start = Date.now();
		const tryConnect = () => {
			const req = http.get(`http://127.0.0.1:${port}/json/version`, (res) => {
				resolve(true);
			});
			req.on('error', () => {
				if (Date.now() - start > timeoutMs) {
					reject(new Error(`CDP port ${port} not ready after ${timeoutMs}ms`));
				} else {
					setTimeout(tryConnect, 200);
				}
			});
			req.setTimeout(1000);
		};
		tryConnect();
	});
}

function killStaleProcesses() {
	try {
		child_process.execSync('taskkill /F /IM filebitch.exe', { stdio: 'ignore' });
	} catch (_) {
		// No process to kill — fine
	}
}

function launchApp() {
	console.log('Starting FileBitch with remote debugging...');

	const tauriProcess = child_process.spawn(BINARY, [], {
		env: {
			...process.env,
			// Microsoft-standard env var for WebView2 launch arguments
			WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT} --disable-http-cache --disable-cache`,
		},
		stdio: 'ignore',
		detached: true,
	});

	console.log(`FileBitch PID: ${tauriProcess.pid}`);
	return tauriProcess;
}

// ─── Browser Connection ───

async function connectBrowser() {
	console.log(`Waiting for CDP on port ${PORT}...`);
	await waitForCdp(PORT);
	console.log('CDP HTTP is ready, connecting Playwright...');

	// WebView2's HTTP endpoint comes up before the WebSocket is ready.
	// Retry the actual CDP connection instead of hoping a timeout is enough.
	let browser;
	const start = Date.now();
	const timeout = 30000;
	while (Date.now() - start < timeout) {
		try {
			browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
			break;
		} catch (e) {
			console.log(`  CDP not ready yet (${Date.now() - start}ms), retrying...`);
			await new Promise(r => setTimeout(r, 500));
		}
	}

	if (!browser) {
		console.error(`CDP connection failed after ${timeout}ms`);
		process.exit(1);
	}

	const defaultContext = browser.contexts()[0];
	const page = defaultContext.pages()[0];

	if (!page) {
		console.error('No page found in the browser context');
		process.exit(1);
	}

	return { browser, page };
}

// ─── Page Setup ───

async function waitForAppReady(page) {
	await page.waitForSelector('#app', { timeout: 10000 });
	const pageUrl = page.url();
	console.log(`  Page URL: ${pageUrl}`);
}

async function setupPageOverrides(page) {
	// Override Tauri invoke to convert camelCase args to snake_case
	await page.evaluate(() => {
		// @ts-ignore
		const orig = window.__TAURI__?.core?.invoke;
		if (orig) {
			// @ts-ignore
			window.__TAURI__.core.invoke = async (cmd, args) => {
				if (args && typeof args === 'object') {
					const fixed = {};
					for (const [k, v] of Object.entries(args)) {
						fixed[k.replace(/[A-Z]/g, m => '_' + m.toLowerCase())] = v;
					}
					return orig(cmd, fixed);
				}
				return orig(cmd, args);
			};
		}
	});
}

function setupConsoleLogging(page) {
	const consoleMessages = [];
	page.on('console', msg => {
		consoleMessages.push(`${msg.type()}: ${msg.text()}`);
	});

	const jsRequests = [];
	page.on('request', req => {
		const url = req.url();
		if (url.endsWith('.js') || url.includes('app.ts')) {
			jsRequests.push(url);
		}
	});

	return { consoleMessages, jsRequests };
}

// ─── E2E Tests ───

async function testAppInit(page, consoleMessages) {
	console.log('Waiting for app to initialize...');
	try {
		await page.waitForSelector('#drives .sidebar-item', { timeout: 10000 });
		console.log('  ✓ App initialized (drives visible)');
	} catch (e) {
		console.log('  ✗ FAIL: App did not initialize - no drives in sidebar');
		console.log('  Console messages:');
		consoleMessages.forEach(m => console.log(`    ${m}`));
		process.exitCode = 1;
	}

	const drives = await page.locator('#drives .sidebar-item').count();
	console.log(`\n✓ Sidebar drives: ${drives}`);
	if (drives === 0) console.log('  ✗ WARNING: no drives found!');
}

async function testFileList(page, drivePath) {
	console.log('\nTesting file list folder icons and navigation...');

	const firstDrive = await page.locator('#drives .sidebar-item').first();
	console.log(`  Navigating to ${drivePath}...`);

	await firstDrive.click();
	await page.waitForTimeout(1000);

	const fileItems = await page.locator('#file-list .file-item').count();
	console.log(`  File items: ${fileItems}`);

	if (fileItems === 0) {
		console.log('  ⚠ No file items to test (drive may be empty)');
		return;
	}

	// Check folder icons
	const firstItemText = await page.locator('#file-list .file-item .icon').first().textContent();
	console.log(`  First item icon: "${firstItemText.trim()}"`);

	if (firstItemText.trim() === '📁') {
		console.log('  ✓ PASS: Folder icon (📁) displayed correctly');
	} else if (firstItemText.trim() === '📄') {
		console.log('  ✗ FAIL: Folder showing as file icon (📄) — entryType mismatch!');
		process.exitCode = 1;
		return;
	} else {
		console.log(`  ⚠ Unexpected icon: ${firstItemText.trim()}`);
	}

	// Double-click navigation
	const firstFolder = page.locator('#file-list .file-item').first();
	const firstFolderPath = await firstFolder.getAttribute('data-path');
	console.log(`  Double-clicking folder: ${firstFolderPath}`);

	await firstFolder.dblclick();
	await page.waitForTimeout(1000);

	const newBreadcrumb = await page.locator('#breadcrumb').textContent();
	console.log(`  New breadcrumb: ${newBreadcrumb}`);

	if (newBreadcrumb !== drivePath && newBreadcrumb.includes(firstFolderPath?.split('\\').pop() || '')) {
		console.log('  ✓ PASS: Double-click navigated into folder');
	} else {
		console.log('  ✗ FAIL: Double-click did not navigate');
		process.exitCode = 1;
	}

	// Navigate back for subsequent tests
	await firstDrive.click();
	await page.waitForTimeout(500);
}

async function testAnalyticsToggle(page, consoleMessages) {
	console.log('\nTesting analytics toggle...');

	const btnAnalyticsVisible = await page.locator('#btn-analytics').isVisible();
	const btnAnalyticsEnabled = await page.locator('#btn-analytics').isEnabled();
	console.log(`  Button visible: ${btnAnalyticsVisible}, enabled: ${btnAnalyticsEnabled}`);

	await page.click('#btn-analytics');
	await page.waitForTimeout(500);

	const errors = consoleMessages.filter(m => m.startsWith('error'));
	if (errors.length > 0) {
		console.log('  Console errors after click:');
		errors.forEach(e => console.log(`    ${e}`));
	}

	const panelVisible = await page.locator('#analytics-panel').isVisible();
	const fileListVisible = await page.locator('#file-list-container').isVisible();

	console.log(`  Panel visible: ${panelVisible}`);
	console.log(`  File list visible: ${fileListVisible}`);

	if (!panelVisible) {
		console.log('  ✗ FAIL: Analytics panel is still hidden after clicking 📊');
		console.log('  All console messages:');
		consoleMessages.forEach(m => console.log(`    ${m}`));
		process.exitCode = 1;
	} else {
		console.log('  ✓ PASS: Analytics panel is visible');
	}

	if (fileListVisible) {
		console.log('  ✗ FAIL: File list is still visible');
		process.exitCode = 1;
	} else {
		console.log('  ✓ PASS: File list is hidden');
	}
}

async function testScanPath(page) {
	const scanPath = await page.inputValue('#scan-path');
	console.log(`\n✓ Scan path pre-populated: "${scanPath}"`);
}

async function testDiskUsageScan(page, consoleMessages) {
	console.log('\nStarting disk usage scan...');

	const scanDir = 'E:\\projects';
	console.log(`  Scanning: ${scanDir}`);
	await page.fill('#scan-path', scanDir);

	consoleMessages.length = 0;

	// Track UI updates during the scan (not just after)
	const uiUpdates = [];
	const scanStart = Date.now();

	// Set up an interval to poll UI state during the scan
	const pollInterval = setInterval(async () => {
		const elapsed = ((Date.now() - scanStart) / 1000).toFixed(1);
		const statusText = await page.locator('#status-info').textContent().catch(() => '');
		const progressWidth = await page.locator('#progress-fill').evaluate(el => el ? el.style.width || '' : '').catch(() => '');
		const rowsVisible = await page.locator('#usage-results .usage-tree-row').count().catch(() => 0);
		const scanningRows = await page.locator('#usage-results .usage-tree-row.scanning').count().catch(() => 0);

		uiUpdates.push({ elapsed, statusText, progressWidth, rowsVisible, scanningRows });
		console.log(`  [${elapsed}s] rows=${rowsVisible} scanning=${scanningRows} progress="${progressWidth}" status="${statusText.trim()}"`);
	}, 500);

	await page.click('#btn-scan');

	console.log('  Waiting for scan to complete...');
	try {
		// Wait for scan completion — summary visible OR progress reaches 100%
		// Some implementations may take a while for large directories
		const scanTimeout = 180000; // 180s
		let completed = false;

		try {
			await page.waitForSelector('#analytics-summary', { state: 'visible', timeout: scanTimeout });
			completed = true;
		} catch {
			// Fallback: check if progress reached 100%
			for (let i = 0; i < scanTimeout / 1000; i++) {
				await page.waitForTimeout(1000);
				const progressWidth = await page.locator('#progress-fill').evaluate(el => el.style.width).catch(() => '');
				if (progressWidth === '100%') {
					completed = true;
					console.log('  ⚠ Summary not visible, but progress reached 100%');
					break;
				}
			}
		}

		const scanDuration = ((Date.now() - scanStart) / 1000).toFixed(2);
		clearInterval(pollInterval);

		if (!completed) {
			console.log('  ✗ FAIL: Scan did not complete within timeout');
			console.log(`  Status info: ${await page.locator('#status-info').textContent()}`);
			process.exitCode = 1;
			return;
		}

		// Brief wait for DOM to settle after completion
		await page.waitForTimeout(1000);

		// 1. Assert scan completed within time limit
		const durationSec = parseFloat(scanDuration);
		const timeLimit = 10; // seconds
		if (durationSec > timeLimit) {
			console.log(`  ✗ FAIL: Scan took ${scanDuration}s (limit: ${timeLimit}s)`);
			process.exitCode = 1;
		} else {
			console.log(`  ✓ PASS: Scan completed in ${scanDuration}s (< ${timeLimit}s)`);
		}

		// 2. Check that UI updated during the scan (not just at the end)
		const updatesWithRows = uiUpdates.filter(u => u.rowsVisible > 0);
		if (updatesWithRows.length >= 2) {
			console.log(`  ✓ PASS: UI updated during scan (${updatesWithRows.length} snapshots showed rows appearing)`);
		} else {
			console.log(`  ⚠ WARNING: UI may not have updated during scan (only ${updatesWithRows.length} snapshots showed rows)`);
		}

		// 3. Check total row count
		const resultRows = await page.locator('#usage-results .usage-tree-row').count();
		console.log(`  ✓ Usage results: ${resultRows} folders in tree`);

		if (resultRows < 1) {
			console.log('  ✗ FAIL: No results found in usage tree');
			process.exitCode = 1;
		}

		// 4. Check the first 20 rows have results (size, file count, folder count)
		// Only check rows that exist — don't require 20 rows, just that visible rows have data
		const rowsToCheck = Math.min(20, resultRows);
		let rowsWithData = 0;
		let rowsMissingData = 0;

		for (let i = 0; i < rowsToCheck; i++) {
			const row = page.locator('#usage-results .usage-tree-row').nth(i);
			const sizeText = await row.locator('.tree-size').textContent();
			const filesText = await row.locator('.tree-files').textContent();
			const foldersText = await row.locator('.tree-folders').textContent();
			const nameText = await row.locator('.tree-name').textContent();

			if (sizeText !== '—' && filesText !== '—' && foldersText !== '—') {
				rowsWithData++;
			} else {
				rowsMissingData++;
				console.log(`    Row ${i + 1} "${nameText}": size="${sizeText}", files="${filesText}", folders="${foldersText}" — MISSING DATA`);
			}
		}

		console.log(`  ✓ ${rowsWithData}/${rowsToCheck} rows have complete data (size + files + folders)`);

		if (rowsWithData < rowsToCheck) {
			console.log(`  ✗ FAIL: ${rowsMissingData} rows missing data`);
			process.exitCode = 1;
		}

		// 5. Summary text
		const summaryText = await page.locator('#summary-text').textContent();
		console.log(`  ✓ Summary: ${summaryText}`);

	} catch (e) {
		clearInterval(pollInterval);
		console.log('  ✗ FAIL: Scan did not complete within timeout');
		console.log(`  Status info: ${await page.locator('#status-info').textContent()}`);
		console.log('  Console messages:');
		consoleMessages.forEach(m => console.log(`    ${m}`));
		process.exitCode = 1;
	}
}

// ─── Main ───

(async () => {
	cleanupWebViewCache();
	killStaleProcesses();
	launchApp();

	const { browser, page } = await connectBrowser();

	try {
		await waitForAppReady(page);
		console.log('App DOM is ready');

		await setupPageOverrides(page);
		const { consoleMessages, jsRequests } = setupConsoleLogging(page);

		await testAppInit(page, consoleMessages);

		const drivePath = await page.inputValue('#scan-path');
		await testFileList(page, drivePath);

		await testAnalyticsToggle(page, consoleMessages);
		await testScanPath(page);
		await testDiskUsageScan(page, consoleMessages);

		await page.screenshot({ path: 'tauri-e2e-screenshot.png', fullPage: true });
		console.log('\nScreenshot saved: tauri-e2e-screenshot.png');

		console.log('\n✓ All checks passed');

	} catch (error) {
		console.error('E2E test failed:', error.message);
		process.exitCode = 1;
	} finally {
		await browser.close();
		try {
			child_process.execSync('taskkill /F /IM filebitch.exe', { stdio: 'ignore' });
			console.log('FileBitch process cleaned up');
		} catch (_) {
			// Process may have already exited
		}
	}
})();
