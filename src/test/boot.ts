import type { TauriMockInvoke } from './helpers';
import { flushPromises } from './helpers';

/**
 * Boots the app by dynamically importing it.
 * Separated from helpers.ts to keep the dynamic import chain
 * isolated for static analysis tools.
 */
export async function bootApp(
  invokeImpl?: (cmd: string, args?: Record<string, unknown>) => unknown,
) {
  const defaultImpl = async (cmd: string) => {
    if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
    return [];
  };

  const { invoke } = await import('@tauri-apps/api/core');
  (invoke as unknown as TauriMockInvoke).mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      return invokeImpl ? invokeImpl(cmd, args) : defaultImpl(cmd);
    },
  );

  // @vite-ignore — dynamic import for test bootstrapping, opaque to static analysis
  await import(/* @vite-ignore */ '../app');
  await flushPromises();
}
