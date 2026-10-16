export function beacon() {
  return '<span class="beacon" aria-hidden="true"><i></i><i></i><i></i></span>';
}

export function escapeHtml(value) {
  const text = document.createElement("span");
  text.textContent = String(value);
  return text.innerHTML;
}

