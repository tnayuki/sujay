import type { ShellHost } from '../host/shell-host';

export interface RuntimeLifecycleHooks {
  onReady: () => void | Promise<void>;
  onWindowAllClosed: () => void | Promise<void>;
  onActivate: () => void | Promise<void>;
  onBeforeQuit: () => void | Promise<void>;
}

export function registerRuntimeLifecycle(host: ShellHost, hooks: RuntimeLifecycleHooks) {
  host.onReady(hooks.onReady);
  host.onWindowAllClosed(hooks.onWindowAllClosed);
  host.onActivate(hooks.onActivate);
  host.onBeforeQuit(hooks.onBeforeQuit);
}
