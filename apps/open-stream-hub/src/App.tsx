import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DeviceFeed, readAutoDetect } from './device-feed';
import { ServiceRunner } from './service-runner';
import { NoticeStore } from './notices';
import { FocusRulesStore, focusStatus } from './focus-rules';
import { DisplaySettingsStore } from './display-settings';
import { DisplaySettingsPanel } from './DisplaySettingsPanel';
import { ObsSettingsStore } from './obs-settings';
import { ObsSettingsPanel } from './ObsSettingsPanel';
import { StreamlabsSettingsStore } from './streamlabs-settings';
import { StreamlabsSettingsPanel } from './StreamlabsSettingsPanel';
import { PermissionsPanel, missingPermissions } from './PermissionsPanel';
import { PermissionsPrompt } from './PermissionsPrompt';
import { SquareEye } from './SquareEye';
import { StartupPanel } from './StartupPanel';
import { keepRunningInMenubar, setKeepRunningInMenubar } from './menubar';
import { useModuleRotations } from './module-rotation';
import { useBaseOrientation } from './base-orientation';
import { UpdatePrompt, UpdatesPanel } from './UpdatesPanel';
import type { UpdateInfo } from './UpdatesPanel';
import { ObsCatalogStore, exposePreviewSeeding, seedPreviewCatalog } from './obs-catalog';
import { setKeyArtwork } from './key-artwork';
import { focusWhenRendered, NoticeRegion, noticeFallbackFocusId } from './NoticeRegion';
import { serviceResumeId, runBlockNotes, RunBlockerNotice, RunHeaderControl, ServicePanel, useRunSession } from './ServicePanel';
import { OnboardingDialog } from './OnboardingDialog';
import { StarterDialog } from './StarterDialog';
import { bundledPacks, parsePack } from './preset-packs';
import type { PresetPack, RejectedPack } from './preset-packs';
import { LiveOfferDialog } from './LiveOfferDialog';
import type { StarterEntry } from './StarterDialog';
import { markStartersOffered, nextModulePrompt, profileModules, shouldOfferStarters } from './starter-model';
import { markModuleControlNoticeShown, ModuleControlNotice, moduleControlNoticeVisible, shouldShowModuleControlNotice } from './ModuleControlNotice';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { AppWindow, CheckCircle2, CircleHelp, FileText, FolderCog, LoaderCircle, Monitor, Moon, Redo2, RotateCcw, Save, Sun, Undo2, Usb, X } from 'lucide-react';
import type { DialDisplayContent, InputPermissions, KeyArtwork, LiveDevice, ObsCatalogNames, Profile, RunMode, ServiceModule } from './contracts';
import { ModalDialog, SelectField } from './controls';
import { BindingsDrawer, DeviceWorkspace } from './DevicesPage';
import { LightingPage } from './LightingPage';
import { LightingStateStore, appearedTargets, restoreLighting } from './lighting-state';
import { ModuleDrawer } from './ModuleDrawer';
import { HistoryMenu } from './HistoryMenu';
import { useT } from './i18n-react';
import { SidebarNav, readNavCollapsed, writeNavCollapsed } from './SidebarNav';
import { RunningProfile, applyRunningProfile } from './RunningProfile';
import { BuiltInProfiles } from './BuiltInProfilesPanel';
import { builtInOf, profileChoices, useHiddenBuiltIns } from './builtin-profiles';
import { drawerTiles, readDrawerOpen, reconcilePicks, writeDrawerOpen } from './module-drawer-model';
import type { DrawerPage, DrawerTile } from './module-drawer-model';
import { useBaseModules } from './base-modules';
import { createModuleSelection } from './module-selection';
import { Inspector } from './Inspector';
import { DiagnosticsPanel } from './DiagnosticsPanel';
import { DemosPage } from './DemosPage';
import { DemoRuntime } from './demos/demo-runtime';
import { demoTilePng } from './demos/TicTacToeScreen';
import { AboutPage, CompatibilityPage, SettingsPage } from './InfoPages';
import { AllowExecutionDialog, MhzImportReport } from './MhzImport';
import { followRunProfile } from './follow-run-profile';
import { readOpenPickers, subscribeOpenPickers } from './ActionPicker';
import { readOpenKeyPickers, subscribeOpenKeyPickers } from './key-capture';
import { isEditing } from './editing-hold';
import { DraftPersister, DraftStore } from './draft-store';
import { isUnsaved, ProfileSession, draftRunBlock } from './profile-session';
import { defaultDisplaySettings } from './display-settings';
import { ProfileManagerSheet } from './ProfileManager';
import { useImportDrop } from './ProfileTransfer';
import { useModifierHeld } from './modifier-held';
import { hint, installShortcuts, isMacPlatform, redoHint, shortcutPages, withHint } from './shortcuts';
import { bindingSearchId } from './BindingsPanel';
import { editKeypadBinding } from './keypad-model';
import { appearanceStorageKey, browserProfile, errorMessage, readAppearance, readAutosaveSetting, markUpdateChecked, markUpdateSeen, readPermissionCheckSetting, readTestWhileEditingSetting, readUpdateCheckSetting, updateAlreadySeen, updateCheckDue, writeUpdateCheckSetting, writeAutosaveSetting, writePermissionCheckSetting, writeTestWhileEditingSetting } from './ui-model';
import type { Appearance, Page } from './ui-model';
import { useDeviceEditor } from './useDeviceEditor';
import type { ConfirmRequest } from './useDeviceEditor';

const desktop = isTauri();
const invokeCommand = <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);
// Browser-preview QA only (never set in the desktop app): design captures seed sample OBS
// listings, because the preview cannot reach Tauri. See ObsCatalogStore.seedPreview.
declare global {
  interface Window {
    __oshObsCatalogSeed?: (names: ObsCatalogNames) => void;
    __oshKeyArtworkSeed?: (control: number, artwork: KeyArtwork) => void;
    __oshKeyMenuSeed?: (control: number, x: number, y: number) => void;
    __oshKeyCopySeed?: (control: number) => void;
    __oshKeySampleSeed?: () => void;
    __oshMacroSeed?: () => void;
    __oshShortcutSeed?: () => void;
    __oshLightingSeed?: (devices: LiveDevice[]) => void;
    __oshDialDisplaySeed?: (content: DialDisplayContent, image?: string) => void;
    __oshStarterSeed?: (entry: StarterEntry, module?: ServiceModule) => void;
    __oshRunModeSeed?: (runMode: RunMode, imported?: boolean) => void;
  }
}
function webStorage(): Storage | null {
  try { return localStorage; } catch { return null; }
}
const mac = isMacPlatform(typeof navigator === 'undefined' ? undefined : navigator);

// Crash-recovery drafts work without storage; the store then does nothing.
function draftPersister() {
  let storage: Storage | null = null;
  try { storage = localStorage; } catch { /* Drafts are simply not kept. */ }
  return new DraftPersister(new DraftStore(storage));
}

function getAppearance(): Appearance {
  try { return readAppearance(localStorage); } catch { return 'system'; }
}

export default function App() {
  const [session] = useState(() => new ProfileSession((command, args) => invoke(command, args), browserProfile, desktop, desktop ? draftPersister() : null));
  const [feed] = useState(() => {
    const created = new DeviceFeed((command, args) => invoke(command, args), (event, handler) => listen(event, handler), desktop);
    try { created.setAutoDetect(readAutoDetect(localStorage)); } catch { /* auto-detect stays on by default */ }
    return created;
  });
  const [runner] = useState(() => new ServiceRunner((command, args) => invoke(command, args), (event, handler) => listen(event, handler), desktop));
  const [demoRuntime] = useState(() => new DemoRuntime({ invoke: (command, args) => invoke(command, args), listen: (event, handler) => listen(event, handler), render: demoTilePng }));
  const [notices] = useState(() => new NoticeStore());
  const [focusRules] = useState(() => new FocusRulesStore((command, args) => invoke(command, args), (event, handler) => listen(event, handler), desktop, runner));
  const [displaySettings] = useState(() => new DisplaySettingsStore((command, args) => invoke(command, args), desktop));
  const [obsSettings] = useState(() => new ObsSettingsStore((command, args) => invoke(command, args), desktop));
  const [streamlabsSettings] = useState(() => new StreamlabsSettingsStore((command, args) => invoke(command, args), desktop));
  const [obsCatalog] = useState(() => new ObsCatalogStore((command, args) => invoke(command, args), desktop));
  // The lighting the Lighting page applies, kept per unit: the rig lights its
  // art with it, and a module that appears again (a replug, or the first
  // inventory after a launch) is sent the same writes once more.
  const [lightingState] = useState(() => new LightingStateStore(webStorage()));
  const deviceFeedState = useSyncExternalStore(feed.subscribe, feed.getState);
  const t = useT();
  const focusState = useSyncExternalStore(focusRules.subscribe, focusRules.getState);
  // The rules are read once at start so the header can say whether automatic switching is on.
  useEffect(() => { if (desktop) void focusRules.load(); }, [focusRules]);
  // The picked module, shared by every page that acts on one module.
  const [moduleSelection] = useState(() => createModuleSelection(webStorage()));
  const bases = useBaseModules(desktop ? invokeCommand : undefined, deviceFeedState.devices.map(device => device.path_hex).join(','));
  const pickableModules = [...bases, ...deviceFeedState.devices];
  // Tell the backend the base's rotation and the rotations the user chose for single modules; it adds each module's
  // connector turn itself, so what a module's screen draws reads upright (applies to runs that start later).
  const baseTurn = useBaseOrientation();
  const rotations = useModuleRotations();
  const turnsKey = JSON.stringify({ base: baseTurn, overrides: rotations });
  useEffect(() => {
    if (!desktop) return;
    invoke('set_module_turns', { preferences: JSON.parse(turnsKey) }).catch(() => {});
  }, [desktop, turnsKey]);
  useModifierHeld(mac);
  const pick = useSyncExternalStore(moduleSelection.subscribe, moduleSelection.getState);
  // A module that appears — the first inventory after a launch, or a replug —
  // is sent the lighting it was last given, so a module keeps its look across
  // a reconnect the way the official app restores it. A unit a run is driving
  // is skipped, and the restores queue up so a burst of inventories (a base
  // replug brings the whole kit back) writes one module at a time.
  const lightingSeen = useRef<string[] | null>(null);
  const restoreChain = useRef<Promise<unknown>>(Promise.resolve());
  const pickableKey = pickableModules.map(device => device.path_hex).join(',');
  useEffect(() => {
    const previous = lightingSeen.current;
    lightingSeen.current = pickableKey.length === 0 ? [] : pickableKey.split(',');
    if (!desktop || !deviceFeedState.inventoryLoaded) return;
    // A path that now answers as another kind of module can never take its old look again, so it is forgotten before
    // anything is restored.
    lightingState.forgetUnappliable(pickableModules);
    const appeared = appearedTargets(previous, pickableModules);
    if (appeared.length === 0) return;
    // A knob or fader module this app has stored nothing for says what it can about its LEDs.
    for (const device of appeared) {
      if ((device.module !== 'm003k' && device.module !== 'm005s') || lightingState.getState().records[device.path_hex]) continue;
      void invokeCommand<{ brightness: number[]; modes: number[] }>('read_led_state', { target: device.path_hex })
        .then(report => lightingState.setReported(device.path_hex, report)).catch(() => {});
    }
    restoreChain.current = restoreChain.current
      .then(() => restoreLighting(lightingState, appeared, invokeCommand, device =>
        device.module === 'm131e' && runner.isActivePath(device.path_hex, { ...device, module: device.module } as LiveDevice)))
      .catch(() => {});
    // Everything the restore reads arrives with the app or the inventory key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [desktop, deviceFeedState.inventoryLoaded, pickableKey, lightingState, runner]);
  // A knob or fader module this app has stored nothing for is drawn from what the module reports, so the report is
  // read again every few seconds (a run answers it too) and the art follows the hardware. Units with a stored look
  // are the app's own writes and are not polled.
  useEffect(() => {
    if (!desktop || !deviceFeedState.inventoryLoaded) return;
    const read = () => {
      if (document.visibilityState === 'hidden') return;
      for (const device of pickableModules) {
        if ((device.module !== 'm003k' && device.module !== 'm005s') || lightingState.getState().records[device.path_hex]) continue;
        void invokeCommand<{ brightness: number[]; modes: number[] }>('read_led_state', { target: device.path_hex })
          .then(report => lightingState.setReported(device.path_hex, report)).catch(() => {});
      }
    };
    const timer = window.setInterval(read, 4000);
    return () => window.clearInterval(timer);
    // The modules are read through the inventory key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [desktop, deviceFeedState.inventoryLoaded, pickableKey, lightingState]);
  useEffect(() => { feed.setBlocker(runner.isActivePath); runner.setLiveSink(feed); feed.connect(); runner.connect(); void runner.syncSessions(); const stopNotices = notices.connect(feed, runner); const stopFocus = focusRules.connect(); return () => { stopFocus(); stopNotices(); runner.setLiveSink(null); feed.dispose(); runner.dispose(); }; }, [feed, runner, notices, focusRules]);
  // A save applies the saved profile to the running sessions that hold it,
  // through the run store that knows them.
  useEffect(() => {
    session.setApplySaved(id => runner.applySavedProfile(id));
    return () => session.setApplySaved(null);
  }, [session, runner]);
  const noticesEnabled = useSyncExternalStore(notices.subscribe, () => notices.getState().enabled);
  // The getting-started guide opens only on request: from Help in the sidebar,
  // from Settings, or from the starter chooser. The starter chooser is the
  // first-run experience instead.
  const [guide, setGuide] = useState(() => ({ open: false, step: 0 }));
  const closeGuide = () => setGuide({ open: false, step: 0 });
  // The starter chooser: the first run (fresh, empty profile storage), the
  // profile manager, and a module that was connected after launch.
  const [starter, setStarter] = useState<{ entry: StarterEntry; module: ServiceModule | null } | null>(null);
  const [liveOfferOpen, setLiveOfferOpen] = useState(false);
  const [liveOfferAck, setLiveOfferAck] = useState(false);
  // The one-time Module control notice: shown once the setting is known to be App and the guide no longer covers it, and
  // recorded as shown only then.
  const [controlNotice, setControlNotice] = useState(() => shouldShowModuleControlNotice(webStorage(), desktop));
  const display = useSyncExternalStore(displaySettings.subscribe, displaySettings.getState);
  // The header's profile picker and the sidebar's running-profile choice are one choice: the profile opens in the editor
  // (after the unsaved-changes check) and becomes the profile of every module.
  const [runningBusy, setRunningBusy] = useState(false);
  const hiddenBuiltIns = useHiddenBuiltIns();
  const applyEverywhere = (id: string, name: string, loaded: Promise<unknown>) => {
    setRunningBusy(true);
    const applied = applyRunningProfile(id, name, runner.getState().runs, { setDefaultProfile: next => displaySettings.setDefaultProfile(next), switchProfile: (sessionId, next, why) => runner.switchProfile(sessionId, next, why) })
      .catch(() => 'The profile could not be applied to the modules.');
    // After the editor's own "Loaded …" notice, so the result of the switch is the one left on screen.
    void Promise.all([loaded, applied]).then(([, notice]) => session.setNotice(notice)).finally(() => setRunningBusy(false));
  };
  const chooseProfile = (id: string) => session.guard('switch profiles', () => {
    const template = builtInOf(id);
    if (template) {
      void session.createFromTemplate(template.profile).then(failure => {
        if (failure) { session.setError(`Could not create the profile: ${failure}`); return; }
        const created = session.getState().draft;
        if (desktop && created.profileId) applyEverywhere(created.profileId, created.profile.name, Promise.resolve());
      });
      return;
    }
    const loaded = session.load(id);
    if (!desktop) return;
    applyEverywhere(id, state.profiles.find(item => item.id === id)?.name ?? 'The profile', loaded);
  });
  // The sidebar's running-profile choice shows the default profile while nothing runs.
  useEffect(() => { if (desktop && !display.defaultProfile.loaded && !display.defaultProfile.busy && !display.defaultProfile.error) void displaySettings.loadDefaultProfile(); }, [desktop, display.defaultProfile.loaded, display.defaultProfile.busy, display.defaultProfile.error, displaySettings]);
  useEffect(() => { if (controlNotice && display.desktop && !display.loaded && !display.busy && !display.error) void displaySettings.load(); }, [controlNotice, displaySettings, display.desktop, display.loaded, display.busy, display.error]);
  useEffect(() => {
    if (previewRunModeSeed === null) return;
    // 'offer' renders the one-time confirmation over an unconfirmed Live
    // store; 'live'/'test' seed confirmed modes.
    if (previewRunModeSeed === 'offer') {
      displaySettings.seedPreview({ ...defaultDisplaySettings, run_mode: 'live', live_offered: false });
      setLiveOfferOpen(true);
      return;
    }
    displaySettings.seedPreview({ ...defaultDisplaySettings, run_mode: previewRunModeSeed.includes('live') ? 'live' : 'test', live_offered: true });
    // The seed is read from the address once; the store is stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const service = useSyncExternalStore(runner.subscribe, runner.getState);
  const showControlNotice = moduleControlNoticeVisible({ pending: controlNotice, guideOpen: guide.open, loaded: display.loaded, moduleControl: display.settings.module_control });
  useEffect(() => { if (showControlNotice) markModuleControlNoticeShown(webStorage()); }, [showControlNotice]);
  const state = useSyncExternalStore(session.subscribe, session.getState);
  // The one-time Live offer: a fresh install (no settings file) reads as Live
  // with the confirmation still owed; after the starter chooser is done, the
  // what-will-run dialog asks once. Until it is answered, the automatic
  // sessions run dry (the backend holds them, with the reason shown).
  useEffect(() => {
    if (!desktop || !state.ready || !display.loaded || starter !== null) return;
    if (display.settings.run_mode !== 'live' || display.settings.live_offered) return;
    setLiveOfferOpen(true);
  }, [desktop, state.ready, display.loaded, display.settings.run_mode, display.settings.live_offered, starter]);
  const [page, setPage] = useState<Page>('workspace');
  const [appearance, setAppearance] = useState<Appearance>(getAppearance);
  const [systemDark, setSystemDark] = useState(() => matchMedia('(prefers-color-scheme: dark)').matches);
  const [confirmation, setConfirmation] = useState<ConfirmRequest | null>(null);
  const editor = useDeviceEditor(session, state, setConfirmation);
  // The shortcuts read the editor of the current render (the closure below would go stale).
  const editorRef = useRef(editor);
  editorRef.current = editor;
  // The module drawer: one picker for the pages that act on a module. Workspace picks a module kind
  // (bindings belong to a kind), Lighting and Artwork a unit; both read the one shared pick, so a
  // pick made on one page is the pick on the next.
  const drawerPage: DrawerPage | null = page === 'workspace' || page === 'lighting' ? page : null;
  const [navCollapsed, setNavCollapsed] = useState(() => readNavCollapsed(webStorage()));
  const [drawerOpen, setDrawerOpen] = useState(() => readDrawerOpen(webStorage()));
  const tiles = drawerPage ? drawerTiles({ page: drawerPage, devices: pickableModules, selectedPath: pick.path, markedPaths: pick.marked, editorKind: editor.selectedModule }) : [];
  function pickModule(tile: DrawerTile) {
    if (tile.path) moduleSelection.select(tile.path);
    if (drawerPage === 'workspace' && tile.module !== 'm0064' && tile.module !== editor.selectedModule) editor.chooseModule(tile.module);
  }
  // The two picks follow each other after the first render: a unit picked elsewhere opens its kind in
  // the editor, and a kind opened in the editor (rig preview, bindings list) moves the shared pick to
  // a unit of that kind. Neither runs on mount, so the saved pick and the editor's start stay as they are.
  const synced = useRef({ path: pick.path, kind: editor.selectedModule });
  useEffect(() => {
    const step = reconcilePicks({ previous: synced.current, pickPath: pick.path, editorKind: editor.selectedModule, devices: pickableModules });
    synced.current = step.next;
    if (step.act === 'editor') editorRef.current.chooseModule(step.kind);
    else if (step.act === 'selection') moduleSelection.select(step.path);
  }, [pick.path, editor.selectedModule, pickableModules, moduleSelection]);
  // The profile manager sheet from the header switcher, the bindings drawer under the canvas and
  // the confirmation that lets an imported profile execute actions (opened from the sheet).
  const [managerOpen, setManagerOpen] = useState(false);
  const [bindingsOpen, setBindingsOpen] = useState(false);
  const [allowExecutionOpen, setAllowExecutionOpen] = useState(false);
  // "Switch to Test mode while editing" (on by default, kept in local storage
  // like automatic saving): the setting the editing hold of the run mode obeys.
  const [checkPermissions, setCheckPermissions] = useState(() => readPermissionCheckSetting(webStorage()));
  const [permissionPrompt, setPermissionPrompt] = useState(false);
  // Once per launch: ask only when macOS reports a permission as off. A failed check asks nothing.
  useEffect(() => {
    if (!desktop || !checkPermissions) return;
    let cancelled = false;
    invoke<InputPermissions>('input_permissions').then(result => { if (!cancelled && missingPermissions(result).length > 0) setPermissionPrompt(true); }).catch(() => {});
    return () => { cancelled = true; };
    // Only at launch: turning the setting on later does not interrupt the user.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [dailyUpdateCheck, setDailyUpdateCheck] = useState(() => readUpdateCheckSetting(webStorage()));
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [updatePrompt, setUpdatePrompt] = useState<UpdateInfo | null>(null);
  // Opt-in, at most once a day, and one message per release. A failed check is silent here; the button shows errors.
  useEffect(() => {
    if (!desktop) return;
    import('@tauri-apps/api/app').then(app => app.getVersion()).then(setAppVersion).catch(() => {});
    const storage = webStorage();
    if (!readUpdateCheckSetting(storage) || !updateCheckDue(storage)) return;
    let cancelled = false;
    invoke<UpdateInfo>('check_for_updates').then(info => {
      markUpdateChecked(storage);
      if (!cancelled && info.available && !updateAlreadySeen(storage, info.latest)) { markUpdateSeen(storage, info.latest); setUpdatePrompt(info); }
    }).catch(() => {});
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [testWhileEditing, setTestWhileEditing] = useState(() => readTestWhileEditingSetting(webStorage()));
  const switcherRef = useRef<HTMLDivElement>(null);
  const { draft, busy, error, notice, noticeAction, pendingIntent, bootstrapError } = state;
  const { profile, profileId, contentHash } = draft;
  const unsaved = isUnsaved(draft);
  // A profile key switches the run's profile on the device; the editor follows it, unless that would replace unsaved changes.
  const followed = useRef<Record<string, number> | null>(null);
  useEffect(() => {
    const step = followRunProfile(service.runs, followed.current, { profileId, unsaved });
    followed.current = step.seen;
    if (step.load) void session.load(step.load);
    else if (step.held) session.setNotice(`A profile key switched the run to ${step.held}. Your unsaved changes stay open here.`);
  }, [service.runs, profileId, unsaved, session]);
  // The open profile (saved, or a draft of it) imported from a .mhz file and not yet allowed to
  // execute actions; the confirmation lives here so it can open over the closed manager sheet.
  const flaggedProfile = state.profiles.find(item => item.id === profileId && item.dry_run_only);
  const disabled = !desktop || !state.ready || busy !== null;
  // Profile files can be dropped anywhere on the workspace to import them.
  const drop = useImportDrop(session, disabled);
  // One run session for the whole app: the header Run control and the Devices-page panel share it.
  // The second arm is the capture seed below (never set in the desktop app): the browser
  // preview has no import flags, so captures can still show the flagged-profile badge.
  // Browser-preview QA only (never set in the desktop app): captures open the
  // preview with ?run-mode=live[,imported] so the header badge and switch show
  // a state the read-only preview cannot reach on its own.
  const previewRunModeSeed = !desktop && import.meta.env.DEV
    ? new URLSearchParams(window.location.search).get('run-mode')
    : null;
  const [previewDryRunOnly, setPreviewDryRunOnly] = useState(() => previewRunModeSeed?.includes('imported') ?? false);
  const dryRunBlock = draftRunBlock(state) ?? (previewDryRunOnly ? 'imported' : null);
  // A start settles the pending automatic save first and runs the saved
  // profile it produced (the session is the truth, not the last render).
  const runSession = useRunSession({
    runner, feed, profile, profileId: unsaved ? null : profileId, sourceProfileId: draft.sourceId ?? profileId,
    desktop, unsaved, displaySettings, dryRunOnly: dryRunBlock !== null, dryRunNote: runBlockNotes[dryRunBlock ?? 'imported'],
    beforeStart: async () => {
      await session.flushAutosave();
      const settled = session.getState().draft;
      return { profile: settled.profile, profileId: isUnsaved(settled) ? null : settled.profileId, sourceProfileId: settled.sourceId ?? settled.profileId, unsaved: isUnsaved(settled) };
    },
  });
  // "Switch to Test mode while editing": while a binding is being edited (an
  // open action picker, the bindings drawer, or unsaved changes) and the
  // setting is on, the mode is held to Test; ending the edit restores it.
  // Once the settings load, a hold left over from a quit or crash mid-edit
  // (the stored held_from marker) restores here too, since no editing is in
  // progress — unless a restored draft makes editing true, and the hold
  // simply continues.
  const pickerOpenCount = useSyncExternalStore(subscribeOpenPickers, readOpenPickers);
  const keyPickerOpenCount = useSyncExternalStore(subscribeOpenKeyPickers, readOpenKeyPickers);
  const editing = isEditing({ unsavedChanges: unsaved, pickerOpen: pickerOpenCount + keyPickerOpenCount > 0, drawerOpen: bindingsOpen });
  useEffect(() => {
    runSession.applyEditingHold(editing, testWhileEditing);
    // The hold reads the current stores through the hook's own closures; only
    // the two inputs and the settings load decide when it runs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editing, testWhileEditing, display.loaded]);
  const connectedModules = deviceFeedState.devices.length;
  const deviceStatus = !desktop ? t('header.status.preview')
    : !deviceFeedState.inventoryLoaded ? deviceFeedState.error ? t('header.status.unavailable') : t('header.status.checking')
      : connectedModules === 0 ? t('header.status.none')
        : t('header.status.connected', { count: connectedModules });
  const deviceStatusKind = !desktop ? 'browser'
    : !deviceFeedState.inventoryLoaded ? deviceFeedState.error ? 'unavailable' : 'checking'
      : connectedModules > 0 ? 'connected' : 'disconnected';

  const focusHeader = focusStatus(focusState, state.profiles);
  const [packReport, setPackReport] = useState<RejectedPack | null>(null);
  // A preset pack becomes a new saved profile (dry run only until allowed). When it names an app, the notice offers a
  // focused-app rule that switches to the profile whenever that app comes to the front.
  function pickPack(pack: PresetPack) {
    session.guard('switch to the preset profile', () => {
      void session.createFromPack(pack.profile).then(failure => {
        if (failure) { session.setError(`Could not create the profile from the preset: ${failure}`); return; }
        setStarter(null);
        setPackReport(null);
        const profileId = session.getState().draft.profileId;
        const bundleId = pack.focus_rule?.bundle_id;
        if (profileId && bundleId) {
          session.setNotice(`Created ${session.getState().draft.profile.name} from the ${pack.name} preset. It runs as a dry run until you review it and allow it to execute actions in the profile manager.`, {
            label: `Switch to it when ${pack.app.name} is in front`,
            run: () => { void focusRules.save({ id: null, app_bundle_id: bundleId, profile_id: profileId }).then(saved => { if (saved && !focusRules.getState().enabled) void focusRules.setEnabled(true); }); },
          });
        }
      });
    });
  }
  // A pack file the user picked is checked like a bundled one; a refused file shows why and creates nothing.
  async function loadPackFile(file: File) {
    if (file.size > 256 * 1024) { setPackReport({ file: file.name, issues: ['A preset file is a small JSON file; this one is larger than 256 KiB.'] }); return; }
    const { pack, issues } = parsePack(await file.text());
    if (pack) pickPack(pack); else setPackReport({ file: file.name, issues });
  }

  useEffect(() => {
    const query = matchMedia('(prefers-color-scheme: dark)');
    const listener = () => setSystemDark(query.matches);
    query.addEventListener('change', listener);
    return () => query.removeEventListener('change', listener);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = appearance === 'system' ? systemDark ? 'dark' : 'light' : appearance;
    try { localStorage.setItem(appearanceStorageKey, appearance); } catch { /* Appearance still works without storage. */ }
  }, [appearance, systemDark]);

  useEffect(() => { if (desktop) void session.bootstrap(); }, [session]);
  // Automatic saving is on by default; the stored choice applies as soon as
  // the session exists, and the toggle keeps both in step.
  useEffect(() => { session.setAutosave(readAutosaveSetting(webStorage())); }, [session]);
  // The first-run chooser opens once profile storage turns out to be fresh
  // and empty. The flag is written when it opens, so a crash while the dialog
  // is up still offers it again next launch; an unreadable store never nags.
  const startersDecided = useRef(false);
  useEffect(() => {
    if (!desktop || !state.ready || startersDecided.current) return;
    startersDecided.current = true;
    if (state.profiles.length || state.unreadable.length) return;
    if (shouldOfferStarters(webStorage(), desktop)) { markStartersOffered(webStorage()); setStarter(current => current ?? { entry: 'first-run', module: null }); }
  }, [desktop, state.ready, state.profiles.length, state.unreadable.length]);
  // A module that appears after the first inventory snapshot is offered a
  // starter once per appearance. The snapshot itself is the baseline: modules
  // connected at launch are not asked about. While profile storage is still
  // opening, the snapshot is held back so the prompt is not raced by the
  // bootstrap. Modules the open profile already uses are skipped (the user
  // has that module set up), as are modules dismissed with "Not now" this
  // session, so a flaky cable does not keep asking.
  const previousDevices = useRef<LiveDevice[] | null>(null);
  const startersDismissed = useRef(new Set<ServiceModule>());
  const draftProfile = state.draft.profile;
  useEffect(() => {
    if (!deviceFeedState.inventoryLoaded || !state.ready) return;
    const previous = previousDevices.current;
    previousDevices.current = deviceFeedState.devices;
    const module = nextModulePrompt(previous, deviceFeedState.devices,
      module => startersDismissed.current.has(module) || profileModules(draftProfile).includes(module));
    if (module !== null) setStarter(current => current ?? { entry: 'module', module });
  }, [deviceFeedState.inventoryLoaded, deviceFeedState.devices, state.ready, draftProfile]);
  // Browser-preview QA hook (dev only): the preview cannot reach Tauri, so design captures seed
  // sample OBS listings through window.__oshObsCatalogSeed. The effect, not the store
  // initializer, points the hook at the store that is actually mounted (StrictMode safe).
  useEffect(() => {
    if (!desktop && import.meta.env.DEV) {
      exposePreviewSeeding(obsCatalog);
      window.__oshObsCatalogSeed = seedPreviewCatalog;
      // Key artwork captures likewise: a saved design cannot be applied in the
      // browser, so seeding one exercises the reopen path directly.
      window.__oshKeyArtworkSeed = (control, artwork) => session.edit(setKeyArtwork(session.getState().draft.profile, ['root'], control, artwork));
      // The key Copy/Paste menu is wired only for the editable desktop app; captures open it
      // through the editor ref, and can seed the clipboard by copying a key.
      window.__oshKeyMenuSeed = (control, x, y) => editorRef.current.openKeyMenu(control, { x, y });
      window.__oshKeyCopySeed = control => editorRef.current.copyKey(control);
      window.__oshKeySampleSeed = () => session.update(latest => {
        let next = editKeypadBinding(latest, ['root'], 2, 'pressed', '', { type: 'toggle_mute' }, 'Mute');
        next = editKeypadBinding(next, ['root'], 3, 'pressed', '', { type: 'timer', seconds: 300, label: 'Break' }, 'Break');
        return editKeypadBinding(next, ['root'], 4, 'pressed', '', { type: 'media', command: 'play_pause' }, 'Play');
      });
      // The macro editor and the shortcut capture field: the preview cannot
      // record or capture, so captures seed a recorded macro and a media-key
      // chord directly.
      window.__oshMacroSeed = () => session.update(latest => editKeypadBinding(latest, ['root'], 5, 'pressed', '', {
        type: 'macro',
        steps: [
          { key: 'c', modifiers: ['command'], delay_before_ms: 0, hold_ms: 80 },
          { key: 'arrow_left', modifiers: ['option', 'shift'], delay_before_ms: 1200, hold_ms: 40 },
          { key: 'keypad_enter', modifiers: [], delay_before_ms: 150, hold_ms: 60 },
        ],
      }, 'Macro'));
      window.__oshShortcutSeed = () => session.update(latest => editKeypadBinding(latest, ['root'], 6, 'pressed', '', { type: 'shortcut', key: 'play_pause', modifiers: [] }, 'Media key'));
      // Lighting captures likewise: the preview has no device inventory, so captures seed one.
      window.__oshLightingSeed = devices => feed.seedPreview(devices);
      // The dial's Display tab: the mode picker works in the preview, but an
      // uploaded picture needs the desktop prepare pipeline, so captures seed one.
      window.__oshDialDisplaySeed = (content, image) => session.update(latest => ({ ...latest, dial_display: image ? { content, image } : { content } }));
      // The starter chooser never opens by itself in the preview (no desktop
      // storage), so captures open it directly.
      window.__oshStarterSeed = (entry, module) => setStarter({ entry, module: module ?? null });
      // The header's run-mode badge and switch likewise: the preview has no
      // display settings to load, so captures seed the shown mode (the URL
      // parameter above is the path captures actually use).
      window.__oshRunModeSeed = (runMode, imported) => {
        displaySettings.seedPreview({ ...defaultDisplaySettings, run_mode: runMode });
        setPreviewDryRunOnly(imported === true);
      };
    }
  }, [session, obsCatalog]);

  // Write any pending crash-recovery copy before the page goes away.
  useEffect(() => {
    const flush = () => session.flushDraft();
    const hidden = () => { if (document.visibilityState === 'hidden') flush(); };
    document.addEventListener('visibilitychange', hidden);
    window.addEventListener('pagehide', flush);
    window.addEventListener('beforeunload', flush);
    return () => { document.removeEventListener('visibilitychange', hidden); window.removeEventListener('pagehide', flush); window.removeEventListener('beforeunload', flush); flush(); };
  }, [session]);

  const latest = useRef({ desktop, ready: state.ready, page });
  latest.current = { desktop, ready: state.ready, page };
  useEffect(() => installShortcuts(document, () => ({
    mac,
    isPageEnabled: page => shortcutPages.includes(page),
    setPage,
    save: () => { if (!latest.current.desktop) session.setNotice('Saving needs the desktop app.'); else if (latest.current.ready) void session.saveOrNote(); },
    duplicate: () => { if (!latest.current.desktop) session.setNotice('Duplicating needs the desktop app.'); else session.requestDuplicate(); },
    undo: () => { session.undo(); },
    redo: () => { session.redo(); },
    // Copy, paste and clear act on the workspace selection (the current render's editor).
    copy: () => latest.current.page === 'workspace' && editorRef.current.copySelection(),
    paste: () => { if (latest.current.page !== 'workspace') return false; editorRef.current.pasteSelection(); return true; },
    delete: () => latest.current.page === 'workspace' && editorRef.current.deleteSelection(),
    // Search opens the bindings drawer on the workspace; elsewhere the browser keeps Cmd/Ctrl+F.
    find: () => {
      if (latest.current.page !== 'workspace') return false;
      setBindingsOpen(true);
      focusWhenRendered(() => document.getElementById(bindingSearchId) as HTMLInputElement | null);
      return true;
    },
  })), [session]);

  // The profile manager sheet closes on Escape and on a click outside the switcher.
  useEffect(() => {
    if (!managerOpen) return;
    const keydown = (event: Event) => { if ((event as KeyboardEvent).key === 'Escape') setManagerOpen(false); };
    const mousedown = (event: MouseEvent) => {
      if (switcherRef.current && event.target instanceof Node && !switcherRef.current.contains(event.target)) setManagerOpen(false);
    };
    document.addEventListener('keydown', keydown);
    document.addEventListener('mousedown', mousedown);
    return () => { document.removeEventListener('keydown', keydown); document.removeEventListener('mousedown', mousedown); };
  }, [managerOpen]);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const destroy = () => {
      getCurrentWindow().destroy().catch(reason => session.setError(`Could not close the window: ${errorMessage(reason)}`));
    };
    getCurrentWindow().onCloseRequested(event => {
      // In the menu bar mode the backend hides the window; quitting arrives as its own event below.
      if (keepRunningInMenubar()) { event.preventDefault(); return; }
      if (!session.requestClose(destroy)) event.preventDefault();
    }).then(remove => { if (disposed) remove(); else unlisten = remove; }).catch(reason => {
      if (!disposed) session.setError(`Could not enable the unsaved-draft close guard: ${errorMessage(reason)}`);
    });
    let unlistenQuit: (() => void) | undefined;
    // Quit (menu bar, app menu or system) runs the same unsaved-draft guard, then ends the app.
    listen('app://quit-requested', () => { if (session.requestClose(destroy)) destroy(); }).then(remove => { if (disposed) remove(); else unlistenQuit = remove; }).catch(() => {});
    invoke<boolean>('menubar_status').then(setKeepRunningInMenubar).catch(() => {});
    return () => { disposed = true; unlisten?.(); unlistenQuit?.(); };
  }, [session]);

  // The menu bar icon's Workspace, Lighting, Demos and Settings items, and its update check.
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    const removers: (() => void)[] = [];
    const add = (name: string, handler: (payload: unknown) => void) => {
      listen(name, event => handler(event.payload)).then(remove => { if (disposed) remove(); else removers.push(remove); }).catch(() => {});
    };
    add('app://navigate', page => { if (page === 'workspace' || page === 'lighting' || page === 'demos' || page === 'settings') setPage(page); });
    add('app://check-updates', () => {
      invoke<UpdateInfo>('check_for_updates').then(setUpdatePrompt).catch(reason => session.setError(`Could not check for updates: ${errorMessage(reason)}`));
    });
    return () => { disposed = true; removers.forEach(remove => remove()); };
  }, [session]);

  const themePicker = <div className="theme-picker" role="group" aria-label={t('theme.group')}>
    {(['system', 'light', 'dark'] as Appearance[]).map(value => <button key={value} aria-pressed={appearance === value}
      className={appearance === value ? 'active' : ''} onClick={() => setAppearance(value)}>
      {value === 'system' ? <Monitor size={15} /> : value === 'light' ? <Sun size={15} /> : <Moon size={15} />}
      {t(`theme.${value}`)}
    </button>)}
  </div>;

  // What the pickers list: the saved profiles and the built-in ones not saved yet. The open profile and the ones that run
  // stay listed even when hidden, so a picker can always show its own value.
  const kept = new Set([...(profileId ? [profileId] : []), ...Object.values(runSession.service.runs).flatMap(run => run.profileId ? [run.profileId] : [])]);
  const choices = profileChoices(state.profiles, hiddenBuiltIns, kept);
  return <div className="app-shell">
    <header className="app-header">
      <div className="header-row">
        <div className="brand"><SquareEye /><span>OpenStreamHub</span><span className="version-label">Desktop</span></div>
        <div className="profile-switcher" ref={switcherRef}><FileText size={18} /><label className="sr-only" htmlFor="active-profile">Active profile</label>
          {desktop ? <SelectField id="active-profile" value={profileId ?? '__new'} disabled={disabled} onChange={event => {
            const id = event.target.value; if (id !== '__new') chooseProfile(id);
          }}>
            {!profileId && <option value="__new">{profile.name}</option>}
            {profileId && !state.profiles.some(item => item.id === profileId) && <option value={profileId}>{profile.name}</option>}
            {choices.map(item => <option key={item.value} value={item.value}>{item.builtIn ? `${item.name} (built-in)` : item.name}</option>)}
          </SelectField> : <span>{profile.name}</span>}
          {unsaved && <span className="draft-dot" title="Unsaved changes" aria-label="Unsaved changes" />}
          <button type="button" className="icon-button manage-profiles" aria-expanded={managerOpen} aria-controls="profile-manager-sheet" aria-label={t('header.manageProfiles')} data-tip={t('header.manageProfiles')} onClick={() => setManagerOpen(value => !value)}><FolderCog size={17} /></button>
          {managerOpen && <ProfileManagerSheet session={session} state={state} desktop={desktop} disabled={disabled} confirm={setConfirmation} mac={mac} focusRules={focusRules} onAllow={() => { setManagerOpen(false); setAllowExecutionOpen(true); }} onStarters={() => { setManagerOpen(false); setStarter({ entry: 'manager', module: null }); }} onClose={() => setManagerOpen(false)} />}
        </div>
        <div className="header-actions"><span className={`device-status-pill ${deviceStatusKind}`} role="status" data-tip={deviceStatus}><Usb size={17} aria-hidden="true" />{deviceStatusKind === 'connected' && <span className="device-status-count" aria-hidden="true">{t('header.status.count', { count: connectedModules })}</span>}<span className="sr-only">{deviceStatus}</span></span>
          {focusHeader && <button type="button" className="focus-status" aria-label={focusHeader.text} data-tip={focusHeader.text} onClick={() => setManagerOpen(true)}><AppWindow size={17} aria-hidden="true" />{focusHeader.matched && <span className="focus-status-name">{focusHeader.matched}</span>}</button>}
          <RunHeaderControl run={runSession} />
          <div className="history-buttons" role="group" aria-label="Change history">
            <button className="icon-button" disabled={disabled || !state.canUndo} data-shortcut={hint('Z', mac)} aria-label={t('header.undo')} data-tip={withHint(state.undoLabels[0] ? t('header.undoNamed', { change: state.undoLabels[0] }) : t('header.undo'), 'Z', mac)} onClick={() => { session.undo(); }}><Undo2 size={18} /></button>
            <button className="icon-button" disabled={disabled || !state.canRedo} data-shortcut={redoHint(mac)} aria-label={t('header.redo')} data-tip={`${state.redoLabels[0] ? t('header.redoNamed', { change: state.redoLabels[0] }) : t('header.redo')} (${redoHint(mac)})`} onClick={() => { session.redo(); }}><Redo2 size={18} /></button>
            <HistoryMenu undo={state.undoLabels} redo={state.redoLabels} disabled={disabled} onUndo={steps => { session.undoSteps(steps); }} onRedo={steps => { session.redoSteps(steps); }}
              undoHint={hint('Z', mac)} redoHint={redoHint(mac)} storage={webStorage()} />
          </div>
          <button className="button" disabled={disabled || !unsaved} data-shortcut={hint('S', mac)} data-tip={withHint(t('header.save'), 'S', mac)} onClick={() => { void session.save(); }}><Save size={17} />{busy === 'Saving profile' ? t('header.saving') : t('header.save')}</button>
        </div>
      </div>
      <RunBlockerNotice run={runSession} />
    </header>
    <div className="app-body">
      <SidebarNav page={page} setPage={setPage} onGuide={() => setGuide({ open: true, step: 0 })} themePicker={themePicker} collapsed={navCollapsed}
        onToggle={() => setNavCollapsed(value => { writeNavCollapsed(webStorage(), !value); return !value; })}
        runningProfile={<RunningProfile profiles={choices.map(item => ({ id: item.value, name: item.builtIn ? `${item.name} (built-in)` : item.name }))} runs={runSession.service.runs} defaultId={display.defaultProfile.profileId} enabled={desktop}
          busy={runningBusy} onChoose={chooseProfile} />} />
      <main className="main-content" id={noticeFallbackFocusId} tabIndex={-1}>
        {!desktop && <div className="browser-banner" role="status"><Monitor size={19} /><div><strong>Desktop app required</strong><span>This browser preview is read-only. Profile storage, validation and hardware access need the native app.</span></div></div>}
        {busy && <div className="operation-status" role="status"><LoaderCircle size={16} className="spinning" />{busy}…</div>}
        {bootstrapError && <div className="alert error" role="alert"><CircleHelp size={19} /><span><strong>Could not open profile storage.</strong> {bootstrapError.message}
          <small className="alert-hint">Check that the profile storage folder exists, is a real folder rather than a link, and is readable by your account{bootstrapError.storagePath ? <>: <code>{bootstrapError.storagePath}</code></> : '.'} Files in it are never changed by a failed start.</small></span>
          <button className="button compact" disabled={busy !== null} onClick={() => { void session.bootstrap(); }}><RotateCcw size={16} />Retry</button></div>}
        {error && <div className="alert error" role="alert"><CircleHelp size={19} /><span>{error}</span><button aria-label="Dismiss error" onClick={() => session.setError(null)}><X size={17} /></button></div>}
        {notice && <div className="alert notice" role="status"><CheckCircle2 size={18} /><span>{notice}</span>{noticeAction && <button className="button compact" onClick={() => { const action = noticeAction; session.setNotice(null); action.run(); }}>{noticeAction.label}</button>}<button aria-label="Dismiss notification" onClick={() => session.setNotice(null)}><X size={17} /></button></div>}
        {showControlNotice && <ModuleControlNotice runMode={runSession.editingHeld ? 'test' : display.settings.run_mode} onDismiss={() => setControlNotice(false)} />}

        {drawerPage && <ModuleDrawer tiles={tiles} open={drawerOpen} onToggle={() => setDrawerOpen(open => { writeDrawerOpen(webStorage(), !open); return !open; })} onPick={pickModule} />}
        {page === 'workspace' && <div className={`device-layout${drop.dragging ? ' drop-target' : ''}`} {...drop.props}>
          <div className="workspace-column">
            <DeviceWorkspace editor={editor} state={state} desktop={desktop} disabled={disabled} setPage={setPage} feed={feed} run={runSession} mac={mac} lighting={lightingState} pickedPath={pick.path} basePath={bases[0]?.path_hex ?? null} onPickUnit={path => moduleSelection.select(path)} />
            <BindingsDrawer open={bindingsOpen} onToggle={() => setBindingsOpen(value => !value)} profile={profile} session={session} editor={editor} disabled={disabled} />
          </div>
          <Inspector editor={editor} session={session} state={state} disabled={disabled} desktop={desktop} invoke={desktop ? invokeCommand : undefined} setPage={setPage} catalog={obsCatalog} liveFader={deviceFeedState.live.m005s?.faders[editor.selection.control]} />
        </div>}
        {page === 'lighting' && <>
          <LightingPage desktop={desktop} invoke={desktop ? invokeCommand : undefined} devices={pickableModules} selection={moduleSelection} runner={runner} storage={webStorage()} lighting={lightingState} />
        </>}
        {page === 'demos' && <DemosPage desktop={desktop} devices={pickableModules} runtime={desktop ? demoRuntime : undefined} />}
        {page === 'compatibility' && <CompatibilityPage />}
        {page === 'about' && <AboutPage />}
        {page === 'settings' && <SettingsPage mac={mac} desktop={desktop} noticesEnabled={noticesEnabled} onNoticesEnabled={on => notices.setEnabled(on)} onShowGuide={() => setGuide({ open: true, step: 0 })} storagePath={state.storagePath} themePicker={themePicker} autosave={state.autosave} onAutosave={on => { writeAutosaveSetting(webStorage(), on); session.setAutosave(on); }} display={<DisplaySettingsPanel store={displaySettings} runner={runner} profiles={state.profiles} testWhileEditing={testWhileEditing} onTestWhileEditing={on => { writeTestWhileEditingSetting(webStorage(), on); setTestWhileEditing(on); }} />} obs={<ObsSettingsPanel store={obsSettings} runner={runner} />} streamlabs={<StreamlabsSettingsPanel store={streamlabsSettings} runner={runner} />} builtIn={<BuiltInProfiles saved={state.profiles} />} startup={<StartupPanel desktop={desktop} invoke={(command, args) => invoke(command, args)} checkPermissions={checkPermissions} onCheckPermissions={on => { writePermissionCheckSetting(webStorage(), on); setCheckPermissions(on); }} />} updates={<UpdatesPanel desktop={desktop} invoke={(command, args) => invoke(command, args)} version={appVersion} daily={dailyUpdateCheck} onDaily={on => { writeUpdateCheckSetting(webStorage(), on); setDailyUpdateCheck(on); }} />} permissions={<PermissionsPanel desktop={desktop} invoke={(command, args) => invoke(command, args)} />} runDetails={<ServicePanel run={runSession} />} diagnostics={<DiagnosticsPanel desktop={desktop} invoke={(command, args) => invoke(command, args)} profile={profile} runner={runner} feed={feed} />} />}
      </main>
    </div>
    <NoticeRegion store={notices} onReview={() => { setPage('workspace'); focusWhenRendered(() => document.getElementById(serviceResumeId)); }} />
    <MhzImportReport result={state.mhzReportOpen ? state.mhzImport : null} onClose={() => session.dismissMhzImport()} />
    <AllowExecutionDialog target={allowExecutionOpen && flaggedProfile && !unsaved ? { name: flaggedProfile.name, profile } : null}
      onCancel={() => setAllowExecutionOpen(false)}
      onConfirm={() => { setAllowExecutionOpen(false); if (flaggedProfile && contentHash) void session.allowExecution(flaggedProfile.id, flaggedProfile.name, contentHash); }} />
    <LiveOfferDialog open={liveOfferOpen} profile={profile} acknowledged={liveOfferAck} setAcknowledged={setLiveOfferAck}
      onConfirm={() => { setLiveOfferOpen(false); setLiveOfferAck(false); void displaySettings.confirmLive(); }}
      onDecline={() => { setLiveOfferOpen(false); setLiveOfferAck(false); void displaySettings.declineLiveOffer(); }} />
    <OnboardingDialog open={guide.open} step={guide.step} mac={mac} onStep={step => setGuide({ open: true, step })} onSkip={closeGuide} onDone={closeGuide} />
    <StarterDialog open={starter !== null} entry={starter?.entry ?? 'first-run'} module={starter?.module ?? null}
      onPick={profile => {
        session.guard('switch to the starter profile', () => {
          void session.createFromTemplate(profile).then(failure => {
            if (failure) session.setError(`Could not create the starter profile: ${failure}`);
            else setStarter(null);
          });
        });
      }}
      onClose={() => {
        if (starter?.entry === 'module' && starter.module) startersDismissed.current.add(starter.module);
        setStarter(null);
      }}
      onGuide={() => { setStarter(null); setGuide({ open: true, step: 0 }); }}
      packs={bundledPacks.packs} rejected={bundledPacks.rejected} packReport={packReport} onPickPack={pickPack} onLoadPack={file => { void loadPackFile(file); }} />
    {updatePrompt && <UpdatePrompt info={updatePrompt} onOpen={() => { void invoke('open_release_page').catch(() => {}); }} onClose={() => setUpdatePrompt(null)} />}
    {permissionPrompt && <PermissionsPrompt invoke={(command, args) => invoke(command, args)} onClose={() => setPermissionPrompt(false)} />}
    <ModalDialog open={confirmation !== null} labelledBy="delete-title" describedBy="delete-message" onCancel={() => setConfirmation(null)}>
      <h2 id="delete-title">{confirmation?.title}</h2><p id="delete-message">{confirmation?.description}</p>
      <div className="dialog-actions"><button className="button" autoFocus onClick={() => setConfirmation(null)}>Cancel</button><button className="button danger" onClick={() => { const request = confirmation; setConfirmation(null); request?.run(); }}>{confirmation?.confirmLabel}</button></div>
    </ModalDialog>
    <ModalDialog open={state.duplicatePrompt} labelledBy="duplicate-title" describedBy="duplicate-message" onCancel={() => { void session.resolveDuplicate(null); }}>
      <h2 id="duplicate-title">Duplicate which version?</h2><p id="duplicate-message">{profile.name} has unsaved changes. The copy replaces them, so the other version is not kept open. Your saved file is never changed.</p>
      <div className="dialog-actions wrap"><button className="button" onClick={() => { void session.resolveDuplicate(null); }}>Cancel</button><button className="button" onClick={() => { void session.resolveDuplicate('saved'); }}>Saved version</button><button className="button primary" autoFocus onClick={() => { void session.resolveDuplicate('draft'); }}>Unsaved changes</button></div>
    </ModalDialog>
    <ModalDialog open={state.restorePrompt !== null} labelledBy="restore-title" describedBy="restore-message" onCancel={() => { /* Escape never discards recovered changes. */ }}>
      <h2 id="restore-title">Restore unsaved changes to {state.restorePrompt?.name}?</h2>
      <p id="restore-message">OpenStreamHub closed before these changes were saved. Restore them to continue editing, or discard them and keep the saved profile.</p>
      {state.restorePrompt?.changedOnDisk && <p className="fine-print" role="note">The saved profile changed on disk after these changes were made. You can still restore them; saving will then refuse until you reload first, so nothing on disk is overwritten.</p>}
      <div className="dialog-actions"><button className="button danger" onClick={() => session.resolveRestore(false)}>Discard</button><button className="button primary" autoFocus onClick={() => session.resolveRestore(true)}>Restore</button></div>
    </ModalDialog>
    <ModalDialog open={pendingIntent !== null} labelledBy="unsaved-title" describedBy="unsaved-message" onCancel={() => session.resolveIntent(false)}>
      <h2 id="unsaved-title">Keep your changes?</h2><p id="unsaved-message">You have unsaved changes. Discard them to {pendingIntent?.label}, or cancel and save your changes first.</p>
      <div className="dialog-actions"><button className="button" autoFocus onClick={() => session.resolveIntent(false)}>Cancel</button><button className="button danger" onClick={() => session.resolveIntent(true)}>Discard changes</button></div>
    </ModalDialog>
  </div>;
}
