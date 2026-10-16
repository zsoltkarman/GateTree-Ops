import { beacon } from "../../shared/ui.js";
import {
  biometricStatus,
  createWorkspace,
  inspectWorkspace,
  unlockWorkspace,
  unlockWorkspaceForDevelopment,
  unlockWorkspaceWithBiometrics,
} from "./workspace-api.js";

const LAST_WORKSPACE_KEY = "pharos.workspace.lastPath";

export function mountWorkspaceGate(root, onUnlocked) {
  let workspace = null;
  let busy = false;

  root.innerHTML = `
    <main class="welcome">
      <section class="welcome-copy">
        <div class="brand brand--welcome">
          ${beacon()}
          <div>
            <strong>PHAROS</strong>
            <small class="acronym-expansion">
              <span><b>P</b>ersistent</span>
              <span><b>H</b>ost</span>
              <span><b>A</b>ccess &amp;</span>
              <span><b>R</b>emote</span>
              <span><b>O</b>perations</span>
              <span><b>S</b>uite</span>
            </small>
          </div>
        </div>
      </section>
      <section class="unlock-panel">
        <div class="panel-heading"><p class="eyebrow">Workspace</p><h2 data-role="title">Choose a location</h2></div>
        <button class="workspace-picker" type="button" data-action="choose">
          <span class="folder-icon" aria-hidden="true"></span>
          <span><strong data-role="name">Select workspace folder</strong><small data-role="path">An empty folder or an existing PHAROS workspace</small></span>
          <b>Choose</b>
        </button>
        <button class="biometric-unlock" type="button" data-action="biometric-unlock" hidden>
          ${fingerprintIcon()}
          <span>Unlock with <b data-role="biometric-label">biometrics</b></span>
        </button>
        <form hidden>
          <label>
            <span>Master password</span>
            <span class="password-control">
              <input data-role="password" type="password" autocomplete="current-password" />
              ${passwordToggle("password")}
            </span>
          </label>
          <label data-role="confirmation-field" hidden>
            <span>Confirm master password</span>
            <span class="password-control">
              <input data-role="confirmation" type="password" autocomplete="new-password" />
              ${passwordToggle("confirmation")}
            </span>
          </label>
          <p class="field-hint" data-role="hint" hidden></p>
          <button class="button button--primary" data-role="submit" disabled>Unlock workspace</button>
        </form>
        <div class="error-box" data-role="error" role="alert" hidden></div>
      </section>
    </main>`;

  const chooseButton = root.querySelector('[data-action="choose"]');
  const form = root.querySelector("form");
  const biometricButton = root.querySelector('[data-action="biometric-unlock"]');
  const password = root.querySelector('[data-role="password"]');
  const confirmation = root.querySelector('[data-role="confirmation"]');
  const confirmationField = root.querySelector('[data-role="confirmation-field"]');
  const submit = root.querySelector('[data-role="submit"]');
  const hint = root.querySelector('[data-role="hint"]');
  const error = root.querySelector('[data-role="error"]');

  root.querySelectorAll("[data-password-toggle]").forEach((toggle) => {
    const input = root.querySelector(`[data-role="${toggle.dataset.passwordToggle}"]`);
    toggle.addEventListener("click", () => {
      const reveal = input.type === "password";
      input.type = reveal ? "text" : "password";
      toggle.classList.toggle("is-revealed", reveal);
      toggle.setAttribute("aria-label", reveal ? "Hide password" : "Show password");
      toggle.setAttribute("title", reveal ? "Hide password" : "Show password");
      input.focus();
    });
  });

  const showError = (cause) => {
    error.textContent = String(cause);
    error.hidden = false;
  };

  const updateForm = () => {
    if (!workspace) return;
    const isNew = workspace.kind === "new";
    const valid = password.value.length > 0 && (!isNew || (password.value.length >= 12 && password.value === confirmation.value));
    submit.disabled = busy || !valid;
    hint.hidden = true;
    if (isNew && password.value.length > 0 && password.value.length < 12) {
      hint.textContent = "Use at least 12 characters.";
      hint.className = "field-hint";
      hint.hidden = false;
    } else if (isNew && confirmation.value.length > 0 && password.value !== confirmation.value) {
      hint.textContent = "Passwords do not match.";
      hint.className = "field-hint field-hint--error";
      hint.hidden = false;
    }
  };

  const loadWorkspace = async (path) => {
    workspace = await inspectWorkspace(path);
    const isNew = workspace.kind === "new";
    if (!isNew) {
      try {
        const session = await unlockWorkspaceForDevelopment(workspace.path);
        localStorage.setItem(LAST_WORKSPACE_KEY, workspace.path);
        onUnlocked(session);
        return;
      } catch {
        // The first debug unlock still requires the master password.
      }
    }
    root.querySelector('[data-role="title"]').textContent = isNew ? "Create workspace" : "Unlock workspace";
    root.querySelector('[data-role="name"]').textContent = workspace.name;
    root.querySelector('[data-role="path"]').textContent = workspace.path;
    confirmationField.hidden = !isNew;
    password.autocomplete = isNew ? "new-password" : "current-password";
    password.placeholder = isNew ? "At least 12 characters" : "Enter your master password";
    submit.textContent = isNew ? "Create encrypted workspace" : "Unlock workspace";
    password.value = "";
    confirmation.value = "";
    form.hidden = false;
    biometricButton.hidden = true;
    if (!isNew) {
      try {
        const biometric = await biometricStatus(workspace.path);
        if (biometric.available && biometric.configured) {
          root.querySelector('[data-role="biometric-label"]').textContent = biometric.label;
          biometricButton.hidden = false;
        }
      } catch {
        biometricButton.hidden = true;
      }
    }
    password.focus();
    updateForm();
  };

  chooseButton.addEventListener("click", async () => {
    error.hidden = true;
    const selected = await window.__TAURI__.dialog.open({ directory: true, multiple: false, canCreateDirectories: true, title: "Choose a PHAROS workspace" });
    if (!selected) return;
    try {
      await loadWorkspace(selected);
    } catch (cause) {
      workspace = null;
      form.hidden = true;
      showError(cause);
    }
  });

  password.addEventListener("input", updateForm);
  confirmation.addEventListener("input", updateForm);
  biometricButton.addEventListener("click", async () => {
    if (!workspace || busy) return;
    busy = true;
    error.hidden = true;
    biometricButton.disabled = true;
    try {
      const session = await unlockWorkspaceWithBiometrics(workspace.path);
      localStorage.setItem(LAST_WORKSPACE_KEY, workspace.path);
      onUnlocked(session);
    } catch (cause) {
      showError(cause);
    } finally {
      busy = false;
      biometricButton.disabled = false;
      updateForm();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!workspace || submit.disabled) return;
    busy = true;
    updateForm();
    error.hidden = true;
    submit.textContent = "Opening…";
    try {
      const session = workspace.kind === "new"
        ? await createWorkspace(workspace.path, password.value)
        : await unlockWorkspace(workspace.path, password.value);
      localStorage.setItem(LAST_WORKSPACE_KEY, workspace.path);
      password.value = "";
      confirmation.value = "";
      onUnlocked(session);
    } catch (cause) {
      showError(cause);
    } finally {
      busy = false;
      if (workspace) submit.textContent = workspace.kind === "new" ? "Create encrypted workspace" : "Unlock workspace";
      updateForm();
    }
  });

  const lastWorkspace = localStorage.getItem(LAST_WORKSPACE_KEY);
  if (lastWorkspace) {
    loadWorkspace(lastWorkspace).catch((cause) => {
      localStorage.removeItem(LAST_WORKSPACE_KEY);
      workspace = null;
      form.hidden = true;
      showError(cause);
    });
  }
}

function fingerprintIcon() {
  return `
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M7.4 9.2a4.9 4.9 0 0 1 9.2 2.3M5.2 12a6.8 6.8 0 0 1 13.5-.2M8.7 12.1c0-4.2 6.5-4.2 6.5 0 0 3.8-1 6.6-2.5 8.3M5.8 15.1c.1 2.2-.4 3.7-1.2 5M11.9 11.4c0 4.2-.7 6.8-2.2 9.1M18.2 14.5c-.1 2.5-.6 4.5-1.5 6" />
    </svg>`;
}

function passwordToggle(target) {
  return `
    <button class="password-toggle" type="button" data-password-toggle="${target}" aria-label="Show password" title="Show password">
      <svg class="eye-open" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M2.8 12s3.2-5 9.2-5 9.2 5 9.2 5-3.2 5-9.2 5-9.2-5-9.2-5Z" />
        <circle cx="12" cy="12" r="2.4" />
      </svg>
      <svg class="eye-closed" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 3l18 18M10.6 7.1A10.4 10.4 0 0 1 12 7c6 0 9.2 5 9.2 5a14 14 0 0 1-2.2 2.7M14.1 14.2A3 3 0 0 1 9.8 10M6.2 6.2A14.8 14.8 0 0 0 2.8 12s3.2 5 9.2 5c1.2 0 2.3-.2 3.3-.6" />
      </svg>
    </button>`;
}

