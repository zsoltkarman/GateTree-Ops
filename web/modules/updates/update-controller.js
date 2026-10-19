import { escapeHtml } from "../../shared/ui.js";

export function createUpdateController({ invoke = window.__TAURI__.core.invoke, listen = window.__TAURI__.event.listen } = {}) {
  let dialog = null;
  let metadata = null;
  let working = false;
  let checking = false;
  let notifyTimer = null;
  let lastFocus = null;
  const dismissedVersions = new Set();
  const status = (message) => { if (dialog) dialog.querySelector('[data-update-status]').textContent = message; };
  const setWorking = (value) => {
    working = value;
    if (!dialog) return;
    dialog.querySelectorAll('button').forEach(button => { button.disabled = value; });
    dialog.setAttribute('aria-busy', String(value));
  };
  function close() {
    if (working) return;
    dialog?.close();
  }
  function show(info = null) {
    if (dialog) { dialog.focus(); return; }
    lastFocus = document.activeElement;
    metadata = info;
    dialog = document.createElement('dialog');
    dialog.className = 'app-update-dialog';
    dialog.setAttribute('aria-labelledby', 'app-update-title');
    dialog.innerHTML = `<header><h2 id="app-update-title">${info?.version ? 'Update available' : 'Check for Updates'}</h2></header>
      <p data-update-version>${info ? escapeHtml(info.version ? `GateTree-Ops ${info.currentVersion} → ${info.version}` : `GateTree-Ops ${info.currentVersion}`) : 'Checking for updates…'}</p>
      <pre data-update-notes>${escapeHtml(info?.notes || '')}</pre>
      <p data-update-status role="status" aria-live="polite"></p>
      <progress data-update-progress hidden></progress>
      <footer><button type="button" data-update-later>${info?.version ? 'Later' : 'Close'}</button>
      <button type="button" data-update-install ${info?.version ? '' : 'hidden'}>Update and restart</button></footer>`;
    document.body.append(dialog);
    dialog.querySelector('[data-update-later]').onclick = close;
    dialog.querySelector('[data-update-install]').onclick = install;
    dialog.addEventListener('cancel', event => { if (working) event.preventDefault(); });
    dialog.addEventListener('close', () => { if (metadata?.version) dismissedVersions.add(metadata.version); dialog.remove(); dialog = null; lastFocus?.focus(); });
    dialog.showModal();
  }
  async function install() {
    if (working || !metadata?.version) return;
    setWorking(true);
    status('Preparing update. Active deployments and SSH sessions must finish first.');
    let unlisten = () => {};
    try {
      unlisten = await listen('gatetree-ops-update-progress', ({ payload }) => {
        const progress = dialog?.querySelector('[data-update-progress]');
        if (!progress) return;
        progress.hidden = false;
        if (payload.total) { progress.max = payload.total; progress.value = payload.downloaded; }
        else { progress.removeAttribute('value'); }
        const labels = { downloading: 'Downloading and verifying signed update…', 'backing-up': 'Backing up your encrypted workspace…', installing: 'Installing update and restarting…' };
        status(labels[payload.phase] || 'Preparing update…');
      });
      await invoke('install_app_update', { expectedVersion: metadata.version });
    } catch (error) {
      status(String(error));
      if (dialog) dialog.querySelector('[data-update-progress]').hidden = true;
    } finally {
      unlisten();
      setWorking(false);
    }
  }
  async function check({ automatic = false } = {}) {
    if (checking || working) return;
    if (!automatic) { if (dialog) { dialog.focus(); return; } show(); }
    checking = true;
    try {
      const info = await invoke('check_app_update');
      if (automatic) {
        if (info.enabled && info.version) {
          const offer = () => {
            if (dismissedVersions.has(info.version)) return;
            if (document.querySelector('dialog[open]')) { notifyTimer = setTimeout(offer, 15000); return; }
            show(info);
          };
          offer();
        }
        return;
      }
      metadata = info;
      if (!dialog) return;
      dialog.querySelector('[data-update-version]').textContent = `GateTree-Ops ${info.currentVersion}${info.version ? ` → ${info.version}` : ''}`;
      dialog.querySelector('[data-update-notes]').textContent = info.notes || '';
      dialog.querySelector('[data-update-install]').hidden = !info.version;
      dialog.querySelector('[data-update-later]').textContent = info.version ? 'Later' : 'Close';
      status(!info.enabled ? 'Automatic updates are available in installed release builds. Debug builds stay unchanged.' : info.version ? 'Your GUI and Agent will update together. Save your work before restarting.' : 'You are using the latest version.');
    } catch (error) { if (!automatic) status(String(error)); }
    finally { checking = false; }
  }
  return { check, start: () => { notifyTimer = setTimeout(() => check({ automatic: true }), 8000); },
    destroy: () => { clearTimeout(notifyTimer); if (!working) close(); } };
}
