import { vi } from 'vitest';

// ─── Types ───

export interface TauriMockInvoke {
  mockReset: () => void;
  mockImplementation: (fn: (...args: any[]) => any) => void;
  mockResolvedValue: (val: any) => void;
  calls: any[][];
}

export interface EventHandler {
  eventName: string;
  handler: (e: any) => void;
}

// ─── Shared State ───

export let registeredHandlers: EventHandler[] = [];

// ─── DOM Factory ───

export function createDom() {
  document.body.innerHTML = `
    <div id="app">
      <div id="toolbar">
        <button id="btn-back" title="Back" disabled>←</button>
        <button id="btn-forward" title="Forward" disabled>→</button>
        <button id="btn-up" title="Up">↑</button>
        <div id="breadcrumb"></div>
        <button id="btn-analytics" title="Analytics">📊</button>
      </div>
      <div id="main-area">
        <aside id="sidebar">
          <div class="sidebar-section">
            <div class="section-header">Drives</div>
            <div class="section-items" id="drives"></div>
          </div>
        </aside>
        <main id="file-list-container">
          <div id="file-list-header">
            <span class="col-name">Name</span>
            <span class="col-size">Size</span>
            <span class="col-date">Modified</span>
          </div>
          <div id="file-list"></div>
        </main>
        <div id="analytics-panel" class="hidden">
          <div id="analytics-toolbar">
            <div class="analytics-tabs">
              <button class="analytics-tab active" data-tab="usage">Disk Usage</button>
              <button class="analytics-tab" data-tab="large-files">Large Files</button>
              <button class="analytics-tab" data-tab="duplicates">Duplicates</button>
              <button class="analytics-tab" data-tab="history">History</button>
            </div>
            <div id="analytics-controls">
              <input type="text" id="scan-path" placeholder="Path to scan..." />
              <button id="btn-scan" class="btn-primary">Scan</button>
              <button id="btn-cancel-scan" class="btn-secondary hidden">Cancel</button>
              <button id="btn-save-snapshot" class="btn-secondary">Save Snapshot</button>
            </div>
          </div>
          <div id="analytics-progress" class="hidden">
            <div class="progress-bar">
              <div class="progress-fill" id="progress-fill"></div>
            </div>
            <span id="progress-text">Scanning...</span>
          </div>
          <div id="analytics-content">
            <div id="tab-usage" class="analytics-tab-content active">
              <div id="usage-results">
                <div class="empty-state">Scan a folder to see disk usage</div>
              </div>
            </div>
            <div id="tab-large-files" class="analytics-tab-content">
              <div id="large-files-results">
                <div class="empty-state">Scan a folder to find large files</div>
              </div>
            </div>
            <div id="tab-duplicates" class="analytics-tab-content">
              <div id="duplicates-results">
                <div class="empty-state">Scan a folder to find duplicates</div>
              </div>
            </div>
            <div id="tab-history" class="analytics-tab-content">
              <div id="history-results">
                <div class="empty-state">Save snapshots to track usage over time</div>
              </div>
            </div>
          </div>
          <div id="analytics-summary" class="hidden">
            <span id="summary-text"></span>
          </div>
        </div>
      </div>
      <div id="statusbar">
        <span id="status-info">Ready</span>
        <span id="status-path"></span>
      </div>
    </div>
  `;
}

// ─── Helpers ───

export async function flushPromises() {
  await new Promise((r) => setImmediate(r));
  await new Promise((r) => setImmediate(r));
}

export function emitEvent(eventName: string, payload: any) {
  const handler = registeredHandlers.find((h) => h.eventName === eventName);
  if (!handler) {
    throw new Error(
      `No handler for "${eventName}". Registered: ${registeredHandlers.map((h) => h.eventName).join(', ')}`,
    );
  }
  handler.handler({ payload });
}

// ─── Boot Helpers ───

export function mockTauriApi() {
  vi.doMock('@tauri-apps/api/core', () => ({
    invoke: vi.fn(),
  }));

  vi.doMock('@tauri-apps/api/event', () => ({
    listen: vi.fn((eventName: string, handler: (e: any) => void) => {
      registeredHandlers.push({ eventName, handler });
      return Promise.resolve(() => {});
    }),
  }));
}

export function resetTauriMocks() {
  vi.resetModules();
  registeredHandlers = [];
  mockTauriApi();
  createDom();
}

