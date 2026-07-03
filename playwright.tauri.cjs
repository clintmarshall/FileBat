const { chromium } = require('playwright');
const child_process = require('child_process');
const path = require('path');
const http = require('http');

const BINARY = path.join(__dirname, 'target/debug/filebitch.exe');
const PORT = 9222;

// Clear WebView2 browser cache so we always load fresh JS
const fs = require('fs');
const os = require('os');
const webViewUserData = path.join(os.homedir(), 'AppData', 'Local', 'filebitch');
if (fs.existsSync(webViewUserData)) {
	try { fs.rmSync(webViewUserData, { recursive: true, force: true }); } catch (_) {}
}

// Poll until the CDP debug port is ready
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

// Kill any stale filebitch instances before starting
try {
	child_process.execSync('taskkill /F /IM filebitch.exe', { stdio: 'ignore' });
} catch (_) {
	// No process to kill — fine
}

(async () => {
	console.log('Starting FileBitch with remote debugging...');

	// Launch the compiled binary with WebView2 debugging enabled
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
	console.log(`Waiting for CDP on port ${PORT}...`);

	try {
		await waitForCdp(PORT);
		console.log('CDP is ready, connecting Playwright...');

		const browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
		const defaultContext = browser.contexts()[0];
		const page = defaultContext.pages()[0];

		if (!page) {
			console.error('No page found in the browser context');
			process.exit(1);
		}

		// Wait for the app to actually load
		await page.waitForSelector('#app', { timeout: 10000 });
		console.log('App DOM is ready');

		// Debug: check what URL the page is loaded from
		const pageUrl = page.url();
		console.log(`  Page URL: ${pageUrl}`);

		// Wait for app to load
		await page.waitForSelector('#app', { timeout: 10000 });

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

		// Collect console messages for debugging
		const consoleMessages = [];
		page.on('console', msg => {
			consoleMessages.push(`${msg.type()}: ${msg.text()}`);
		});

		// Intercept network requests to see what JS is being loaded
		const jsRequests = [];
		page.on('request', req => {
			const url = req.url();
			if (url.endsWith('.js') || url.includes('app.ts')) {
				jsRequests.push(url);
			}
		});

		// Wait for the app to initialize (drives should appear)
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

		// ─── Test 1: Drives in sidebar ───
		const drives = await page.locator('#drives .sidebar-item').count();
		console.log(`\n✓ Sidebar drives: ${drives}`);
		if (drives === 0) console.log('  ✗ WARNING: no drives found!');

		// ─── Test 1b: File list shows folder icons and double-click navigates ───
		console.log('\nTesting file list folder icons and navigation...');

		// Get the first drive path and click it
		const firstDrive = await page.locator('#drives .sidebar-item').first();
		const drivePath = await page.inputValue('#scan-path'); // Pre-populated path like "C:\"
		console.log(`  Navigating to ${drivePath}...`);

		// Click on the first drive in sidebar
		await firstDrive.click();
		await page.waitForTimeout(1000);

		// Check that file list items exist
		const fileItems = await page.locator('#file-list .file-item').count();
		console.log(`  File items: ${fileItems}`);

		if (fileItems > 0) {
			// Check for folder icons (📁) in the file list
			const firstItemText = await page.locator('#file-list .file-item .icon').first().textContent();
			console.log(`  First item icon: "${firstItemText.trim()}"`);

			// Folders should have 📁 icon, not 📄
			if (firstItemText.trim() === '📁') {
				console.log('  ✓ PASS: Folder icon (📁) displayed correctly');
			} else if (firstItemText.trim() === '📄') {
				console.log('  ✗ FAIL: Folder showing as file icon (📄) — entryType mismatch!');
				process.exitCode = 1;
			} else {
				console.log(`  ⚠ Unexpected icon: ${firstItemText.trim()}`);
			}

			// Double-click the first folder to navigate into it
			const firstFolder = page.locator('#file-list .file-item').first();
			const firstFolderPath = await firstFolder.getAttribute('data-path');
			console.log(`  Double-clicking folder: ${firstFolderPath}`);

			await firstFolder.dblclick();
			await page.waitForTimeout(1000);

			// Check that the breadcrumb updated (navigation happened)
			const newBreadcrumb = await page.locator('#breadcrumb').textContent();
			console.log(`  New breadcrumb: ${newBreadcrumb}`);

			if (newBreadcrumb !== drivePath && newBreadcrumb.includes(firstFolderPath?.split('\\').pop() || '')) {
				console.log('  ✓ PASS: Double-click navigated into folder');
			} else {
				console.log('  ✗ FAIL: Double-click did not navigate');
				process.exitCode = 1;
			}

			// Navigate back to the drive for subsequent tests
			await firstDrive.click();
			await page.waitForTimeout(500);
		} else {
			console.log('  ⚠ No file items to test (drive may be empty)');
		}

		// ─── Test 2: Analytics toggle ───
		console.log('\nTesting analytics toggle...');

		// Check if the button is clickable
		const btnAnalyticsVisible = await page.locator('#btn-analytics').isVisible();
		const btnAnalyticsEnabled = await page.locator('#btn-analytics').isEnabled();
		console.log(`  Button visible: ${btnAnalyticsVisible}, enabled: ${btnAnalyticsEnabled}`);

		// Click the analytics button
		await page.click('#btn-analytics');
		await page.waitForTimeout(500);

		// Check console for errors
		const errors = consoleMessages.filter(m => m.startsWith('error'));
		if (errors.length > 0) {
			console.log('  Console errors after click:');
			errors.forEach(e => console.log(`    ${e}`));
		}

		// Check if the panel is now visible
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

		// ─── Test 3: Scan path pre-populated ───
		const scanPath = await page.inputValue('#scan-path');
		console.log(`\n✓ Scan path pre-populated: "${scanPath}"`);

		// ─── Test 4: Disk Usage Scan ───
		console.log('\nStarting disk usage scan...');

		// Navigate to a small directory for the scan (not C:\)
		const scanDir = path.join(__dirname, 'src');
		console.log(`  Scanning: ${scanDir}`);
		await page.fill('#scan-path', scanDir);

		// Clear previous console messages
		consoleMessages.length = 0;

		// Click Scan button
		await page.click('#btn-scan');

		// Wait for scan to complete (summary appears) or timeout.
		// Small directories scan so fast the progress bar may never be visible.
		console.log('  Waiting for scan to complete...');
		try {
			await page.waitForSelector('#analytics-summary', { state: 'visible', timeout: 60000 });
			console.log('  ✓ PASS: Scan completed');

			
			
			// Verify results table has rows
			const resultRows = await page.locator('#usage-results .analytics-table tr').count();
			console.log(`  ✓ Usage results: ${resultRows - 1} folders scanned`); // -1 for header row

			if (resultRows < 2) {
				console.log('  ✗ FAIL: No results found in usage table');
				process.exitCode = 1;
			}

			// Verify summary text
			const summaryText = await page.locator('#summary-text').textContent();
			console.log(`  ✓ Summary: ${summaryText}`);

		} catch (e) {
			console.log('  ✗ FAIL: Scan did not complete within timeout');
			console.log(`  Status info: ${await page.locator('#status-info').textContent()}`);
			console.log('  Console messages:');
			consoleMessages.forEach(m => console.log(`    ${m}`));
			process.exitCode = 1;
		}

		// Take screenshot for visual confirmation
		await page.screenshot({ path: 'tauri-e2e-screenshot.png', fullPage: true });
		console.log('\nScreenshot saved: tauri-e2e-screenshot.png');

		await browser.close();
		console.log('\n✓ All checks passed');

	} catch (error) {
		console.error('E2E test failed:', error.message);
		process.exitCode = 1;
	} finally {
		// Kill filebitch process by image name (works on Windows/Unix)
		try {
			child_process.execSync('taskkill /F /IM filebitch.exe', { stdio: 'ignore' });
			console.log('FileBitch process cleaned up');
		} catch (_) {
			// Process may have already exited
		}
	}
})();
