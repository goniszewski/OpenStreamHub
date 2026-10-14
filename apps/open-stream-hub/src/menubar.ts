// Whether closing the window hides it in the menu bar instead of quitting. The backend decides (and hides the
// window); this copy only tells the close handler not to run the quit path as well. It is loaded at start and
// updated by Settings.
let keepRunning = false;

export const keepRunningInMenubar = () => keepRunning;
export const setKeepRunningInMenubar = (on: boolean) => { keepRunning = on; };
