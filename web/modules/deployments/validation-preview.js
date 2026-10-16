export const PREVIEW_SETTINGS = [
  ["Role", "Role name"], ["Environment", "Environment"], ["Location", "Location"],
  ["Strategy", "Strategy"], ["Running user", "Running User"], ["Galaxy version", "Galaxy version"],
  ["Docker host", "AF Docker host"], ["Docker repository", "AF Docker repo"], ["Docker image", "AF Docker image"],
];

// Accept both a single debug string and older msg arrays, merging
// fields from separate tasks (including IMAGE_VERSION) by inventory hostname.
export function parseValidationPreview(output) {
  const text = String(output ?? "").replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
  const hosts = new Map();
  const messages = /"msg"\s*:\s*("(?:\\.|[^"\\])*"|\[(?:[^\]"\\]|"(?:\\.|[^"\\])*")*\])/g;
  for (const match of text.matchAll(messages)) {
    let message;
    try { message = JSON.parse(match[1]); } catch { continue; }
    if (Array.isArray(message)) message = message.filter((value) => typeof value === "string").join("\n");
    if (typeof message !== "string") continue;
    const fields = previewFields(message);
    if (!Object.keys(fields).length) continue;
    const header = [...text.slice(0, match.index).matchAll(/(?:ok|changed):\s*\[([^\]]+)\]\s*=>/g)].at(-1);
    const host = header?.[1] ?? fields["Ansible hostname"];
    if (!host) continue;
    hosts.set(host, { ...hosts.get(host), ...fields });
  }
  return [...hosts.entries()]
    .filter(([, fields]) => fields["Role name"] || fields["Ansible hostname"])
    .map(([host, fields]) => ({ ...fields, "Ansible hostname": fields["Ansible hostname"] || host }))
    .sort((a, b) => a["Ansible hostname"].localeCompare(b["Ansible hostname"]));
}

function previewFields(message) {
  const fields = {};
  let previousKey = null;
  for (const rawLine of message.replace(/\\n/g, "\n").replace(/\\r/g, "\r").split(/\r?\n/)) {
    const line = rawLine.trim();
    const separator = line.indexOf(":");
    if (separator > 0) {
      const key = line.slice(0, separator).trim();
      // Ignore arbitrary debug prose; accept the preview's named fields and
      // role variables such as KAFKA_VERSION or JDK_VERSION.
      if (PREVIEW_SETTINGS.some(([, field]) => field === key)
        || ["Ansible hostname", "Ansible ssh host", "IMAGE_VERSION"].includes(key)
        || /^[A-Z][A-Z0-9_]+$/.test(key)) {
        fields[key] = line.slice(separator + 1).trim();
        previousKey = key;
      } else { previousKey = null; }
    } else if (line && previousKey && fields[previousKey] === "") {
      fields[previousKey] = line;
    }
  }
  return fields;
}
