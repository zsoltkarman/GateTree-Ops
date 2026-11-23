import { escapeHtml } from '../../shared/ui.js';

export function tunnelRow(tunnel = { sourcePort: 13306, destinationHost: '127.0.0.1', destinationPort: 3306 }) {
  return `<div class="ssh-tunnel-row" data-role="tunnel-row">
    <label>Source port<input aria-label="Local source port" data-field="sourcePort" type="number" min="1" max="65535" required value="${tunnel.sourcePort}" /></label>
    <label>Destination host<input aria-label="Destination host on the SSH side" data-field="destinationHost" required autocomplete="off" spellcheck="false" value="${escapeHtml(tunnel.destinationHost)}" /></label>
    <label>Destination port<input aria-label="Destination port" data-field="destinationPort" type="number" min="1" max="65535" required value="${tunnel.destinationPort}" /></label>
    <button type="button" data-action="remove-tunnel" aria-label="Remove tunnel">Remove</button>
  </div>`;
}

export function tunnelEditor(connection) {
  return `<details class="ssh-tunnel-editor"><summary>SSH tunnels · ${(connection.localTunnels ?? []).length} local</summary>
    <form data-role="ssh-tunnels-form" data-connection-id="${escapeHtml(connection.id)}">
      <p>Forward a port on this computer to a destination reached from the SSH server. Local listeners use 127.0.0.1.</p>
      <div data-role="tunnel-rows">${(connection.localTunnels ?? []).map(tunnelRow).join('')}</div>
      <div class="ssh-tunnel-actions"><button type="button" data-action="add-tunnel">Add local tunnel</button><button type="submit">Save tunnels</button></div>
      <p>Saving tunnels selects Terminal login (OpenSSH). Reconnect SSH to apply changes; keep its terminal open while using the tunnel. Password / MFA prompts appear there. A busy source port causes SSH to exit with an error.</p>
      <p data-role="tunnel-feedback" role="status"></p>
    </form>
  </details>`;
}

export function readTunnelRows(form) {
  const tunnels = [...form.querySelectorAll('[data-role="tunnel-row"]')].map(row => ({
    sourcePort: Number(row.querySelector('[data-field="sourcePort"]').value),
    destinationHost: row.querySelector('[data-field="destinationHost"]').value.trim(),
    destinationPort: Number(row.querySelector('[data-field="destinationPort"]').value),
  }));
  const ports = new Set();
  if (tunnels.length > 32) throw new Error('At most 32 local tunnels are supported.');
  for (const tunnel of tunnels) {
    if (![tunnel.sourcePort, tunnel.destinationPort].every(port => Number.isInteger(port) && port >= 1 && port <= 65535) || !tunnel.destinationHost) throw new Error('Enter a destination host and ports between 1 and 65535.');
    if (ports.has(tunnel.sourcePort)) throw new Error('Each tunnel needs a different local source port.');
    ports.add(tunnel.sourcePort);
  }
  return tunnels;
}

export function bindTunnelEditor(root, save) {
  const form = root.querySelector('[data-role="ssh-tunnels-form"]');
  if (!form) return;
  form.addEventListener('click', event => {
    const button = event.target.closest('button');
    if (button?.dataset.action === 'add-tunnel') form.querySelector('[data-role="tunnel-rows"]').insertAdjacentHTML('beforeend', tunnelRow());
    if (button?.dataset.action === 'remove-tunnel') button.closest('[data-role="tunnel-row"]').remove();
  });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    const feedback = form.querySelector('[data-role="tunnel-feedback"]');
    const button = form.querySelector('[type="submit"]');
    button.disabled = true;
    try {
      await save(form.dataset.connectionId, readTunnelRows(form));
      root.querySelector('.ssh-tunnel-editor summary').textContent = `SSH tunnels · ${form.querySelectorAll('[data-role="tunnel-row"]').length} local`;
      feedback.textContent = 'Saved. Open a new SSH session to use these tunnels.';
    } catch (error) {
      feedback.textContent = String(error.message ?? error);
    } finally { button.disabled = false; }
  });
}
