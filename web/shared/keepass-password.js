export function requestKeePassPassword(root, { signal, restoring = false, error = "" } = {}) {
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "queue-password-dialog";
    dialog.innerHTML = `<form method="dialog">
      <strong>Unlock KeePassXC</strong>
      <small>KeePass credentials are read into Agent memory. The master password is not saved.</small>
      <p class="agent-error" data-role="password-error" hidden></p>
      <label><span>Master password</span><input type="password" autocomplete="current-password" required></label>
      <div><button class="connection-primary-action" type="submit">${restoring ? "Unlock" : "Unlock and Start"}</button><button type="button" data-action="cancel-password">Cancel</button></div>
    </form>`;
    const errorMessage = dialog.querySelector('[data-role="password-error"]');
    errorMessage.textContent = error;
    errorMessage.hidden = !error;
    root.append(dialog);
    const input = dialog.querySelector("input");
    dialog.querySelector("form").addEventListener("submit", (event) => {
      event.preventDefault();
      if (input.reportValidity()) dialog.close("unlock");
    });
    dialog.querySelector('[data-action="cancel-password"]').addEventListener("click", () => dialog.close("cancel"));
    const cancel = () => dialog.close("cancel");
    signal?.addEventListener("abort", cancel, { once: true });
    dialog.addEventListener("close", () => {
      signal?.removeEventListener("abort", cancel);
      const value = dialog.returnValue === "unlock" ? input.value : null;
      input.value = "";
      dialog.remove();
      resolve(value);
    }, { once: true });
    dialog.showModal();
    requestAnimationFrame(() => input.focus());
  });
}
