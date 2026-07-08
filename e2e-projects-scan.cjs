const { chromium } = require('playwright');
const child_process = require('child_process');
const path = require('path');
const http = require('http');
const fs = require('fs');
const os = require('os');

const BINARY = path.join(__dirname, 'target/debug/filebitch.exe');
const PORT = 9222;

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
  } catch (_) {}
}

function launchApp() {
  console.log('Starting FileBitch with remote debugging...');
  const tauriProcess = child_process.spawn(BINARY, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT} --disable-http-cache --disable-cache`,
    },
    stdio: 'ignore',
    detached: true,
  });
  console.log(`FileBitch PID: ${tauriProcess.pid}`);
  return tauriProcess;
}

(async () => {
  // cleanupWebViewCache();  // Skip — forces WebView2 re-download
  killStaleProcesses();
  await new Promise(r => setTimeout(r, 2000));  // Give processes time to die
  launchApp();

  // Wait for CDP
  console.log(`Waiting for CDP on port ${PORT}...`);
  await waitForCdp(PORT, 30000);
  console.log('CDP ready');

  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
  const defaultContext = browser.contexts()[0];
  const page = defaultContext.pages()[0];

  if (!page) { console.error('No page found'); process.exit(1); }

  await page.waitForSelector('#app', { timeout: 10000 });
  await page.waitForSelector('#drives .sidebar-item', { timeout: 10000 });
  console.log('App initialized');

  // Override Tauri invoke (camelCase → snake_case)
  await page.evaluate(() => {
    const orig = window.__TAURI__?.core?.invoke;
    if (orig) {
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

  // Capture console
  const consoleMessages = [];
  page.on('console', msg => {
    consoleMessages.push(`${msg.type()}: ${msg.text()}`);
  });

  // Click analytics toggle
  await page.click('#btn-analytics');
  await page.waitForTimeout(500);

  // Set scan path to E:\projects
  const scanDir = 'E:\\projects';
  console.log(`Scanning: ${scanDir}`);
  await page.fill('#scan-path', scanDir);

  consoleMessages.length = 0;
  await page.click('#btn-scan');

  // Wait for scan to complete (up to 5 minutes)
  console.log('Waiting for scan to complete...');
  let scanCompleted = false;
  try {
    await page.waitForSelector('#analytics-summary', { state: 'visible', timeout: 600000 });
    scanCompleted = true;
    console.log('✓ Scan completed');
  } catch (e) {
    console.log('✗ Scan did not complete within timeout');
  }

  // Check tree rows
  const rows = await page.locator('#usage-results .usage-tree-row').count();
  console.log(`Tree rows in DOM: ${rows}`);

  // Get each row's data-path and stats
  const rowEls = await page.locator('#usage-results .usage-tree-row').all();
  for (const el of rowEls) {
    const dp = await el.getAttribute('data-path');
    const name = await el.locator('.tree-name').textContent();
    const size = await el.locator('.tree-size').textContent();
    const files = await el.locator('.tree-files').textContent();
    const folders = await el.locator('.tree-folders').textContent();
    const toggle = await el.locator('.tree-toggle').textContent();
    console.log(`  Row: path="${dp}" name="${name}" size="${size}" files="${files}" folders="${folders}" toggle="${toggle}"`);
  }

  // Get summary
  try {
    const summaryText = await page.locator('#summary-text').textContent();
    console.log(`Summary: ${summaryText}`);
  } catch(e) {}

  // Check knownChildren state via JS
  const knownChildrenInfo = await page.evaluate(() => {
    // knownChildren is a module-level variable, not accessible directly
    // Check the DOM for expand buttons
    const rows = document.querySelectorAll('.usage-tree-row');
    const result = [];
    rows.forEach(r => {
      const toggle = r.querySelector('.tree-toggle');
      result.push({
        path: r.getAttribute('data-path'),
        toggleText: toggle ? toggle.textContent : 'none',
        toggleClass: toggle ? toggle.className : 'none',
        disabled: toggle ? toggle.classList.contains('disabled') : true,
      });
    });
    return result;
  });
  console.log('\nRow expand state:');
  knownChildrenInfo.forEach(r => {
    console.log(`  ${r.path}: toggle="${r.toggleText}" disabled=${r.disabled}`);
  });

  // Check scanResults state
  const scanResultsInfo = await page.evaluate(() => {
    // scanResults is module-level — check via DOM
    const rows = document.querySelectorAll('.usage-tree-row');
    return {
      rowCount: rows.length,
      rowsWithStats: Array.from(rows).filter(r => {
        const size = r.querySelector('.tree-size');
        return size && size.textContent !== '—';
      }).length,
      rowsWithoutStats: Array.from(rows).filter(r => {
        const size = r.querySelector('.tree-size');
        return size && size.textContent === '—';
      }).length,
    };
  });
  console.log(`\nStats: ${scanResultsInfo.rowCount} rows, ${scanResultsInfo.rowsWithStats} with stats, ${scanResultsInfo.rowsWithoutStats} without`);

  // Log console messages
  console.log('\nConsole messages:');
  consoleMessages.forEach(m => console.log(`  ${m}`));

  // Screenshot
  await page.screenshot({ path: 'e2e-projects-scan.png', fullPage: true });
  console.log('\nScreenshot saved: e2e-projects-scan.png');

  await browser.close();
  try { child_process.execSync('taskkill /F /IM filebitch.exe', { stdio: 'ignore' }); } catch (_) {}
})();
