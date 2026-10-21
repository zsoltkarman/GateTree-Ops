import { getLocalAnsibleSettings, setLocalAnsibleFolder } from "./deployments-api.js";

export function mountLocalAnsibleSource(root, { onChange = () => {}, load = getLocalAnsibleSettings, save = setLocalAnsibleFolder, choose = () => window.__TAURI__.dialog.open({ directory: true, multiple: false, canCreateDirectories: false, title: "Choose the Ansible project folder containing inventory" }) } = {}) {
  let disposed = false;
  root.classList.add("local-ansible-source");
  root.innerHTML = `<div><strong>Local Ansible folder</strong><span data-source-path>Loading…</span><small>Saved on this computer · Project folder containing inventory</small></div><button type="button" data-source-choose disabled>Choose folder</button><p data-source-error role="alert" hidden></p>`;
  const alive = () => !disposed && root.isConnected;
  const button = root.querySelector("button");
  function display(settings) {
    root.querySelector("[data-source-path]").textContent = settings.path || "No folder selected";
    root.querySelector("[data-source-path]").title = settings.path || "";
    button.textContent = settings.path ? "Change folder" : "Choose folder";
  }
  function error(cause) {
    const el = root.querySelector("[data-source-error]"); el.textContent = String(cause); el.hidden = false;
  }
  button.addEventListener("click", async () => {
    button.disabled = true; root.querySelector("[data-source-error]").hidden = true;
    try {
      const path = await choose(); if (!alive() || !path) return;
      const settings = await save(path); if (!alive()) return;
      display(settings); onChange(settings);
      window.dispatchEvent(new CustomEvent("local-ansible-folder-changed", { detail: settings }));
    } catch (cause) { if (alive()) error(cause); }
    finally { if (alive()) button.disabled = false; }
  });
  void (async () => {
    try { const settings = await load(); if (alive()) { display(settings); onChange(settings); } }
    catch (cause) { if (alive()) { display({}); error(cause); } }
    finally { if (alive()) button.disabled = false; }
  })();
  return () => { disposed = true; };
}
