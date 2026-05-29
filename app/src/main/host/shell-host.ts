import type { App, BrowserWindowConstructorOptions, BaseWindow, BrowserWindow as ElectronBrowserWindow, Menu as ElectronMenu, MenuItemConstructorOptions } from 'electron';

type BrowserWindowCtor = {
  new (options?: BrowserWindowConstructorOptions): ElectronBrowserWindow;
  getAllWindows(): BaseWindow[];
};

type MenuApi = {
  buildFromTemplate(template: MenuItemConstructorOptions[]): ElectronMenu;
  setApplicationMenu(menu: ElectronMenu | null): void;
};

export type ShellPathKey = 'music';

export interface ShellHost {
  readonly appName: string;
  readonly platform: NodeJS.Platform;
  getPath(name: ShellPathKey): string;
  quit(): void;
  onReady(listener: () => void | Promise<void>): void;
  onWindowAllClosed(listener: () => void | Promise<void>): void;
  onActivate(listener: () => void | Promise<void>): void;
  onBeforeQuit(listener: () => void | Promise<void>): void;
  shouldOpenMainWindow(): boolean;
  createBrowserWindow(options: BrowserWindowConstructorOptions): ElectronBrowserWindow;
  buildMenuFromTemplate(template: MenuItemConstructorOptions[]): ElectronMenu;
  setApplicationMenu(menu: ElectronMenu | null): void;
  getAppMetrics(): ReturnType<App['getAppMetrics']>;
}

export function createElectronShellHost(deps: {
  app: App;
  BrowserWindow: BrowserWindowCtor;
  Menu: MenuApi;
}): ShellHost {
  const { app, BrowserWindow, Menu } = deps;

  return {
    get appName() {
      return app.name;
    },
    get platform() {
      return process.platform;
    },
    getPath(name) {
      return app.getPath(name);
    },
    quit() {
      app.quit();
    },
    onReady(listener) {
      app.whenReady().then(listener).catch((error) => {
        console.error('[ShellHost] ready hook failed', error);
      });
    },
    onWindowAllClosed(listener) {
      app.on('window-all-closed', listener);
    },
    onActivate(listener) {
      app.on('activate', listener);
    },
    onBeforeQuit(listener) {
      app.on('before-quit', listener);
    },
    shouldOpenMainWindow() {
      return BrowserWindow.getAllWindows().length === 0;
    },
    createBrowserWindow(options) {
      return new BrowserWindow(options);
    },
    buildMenuFromTemplate(template) {
      return Menu.buildFromTemplate(template);
    },
    setApplicationMenu(menu) {
      Menu.setApplicationMenu(menu);
    },
    getAppMetrics() {
      return app.getAppMetrics();
    },
  };
}
