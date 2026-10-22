// WKWebView can omit the keypress that xterm relies on for Option characters.
// Use the character resolved by macOS, so this also follows the active layout.
export function configureTerminalKeyboard(terminal, platform = navigator.platform) {
  if (!/Mac/i.test(platform)) return;
  terminal.options.macOptionIsMeta = false;
  let handledOption = false;
  terminal.attachCustomKeyEventHandler(event => {
    if (event.type === "keyup") { handledOption = false; return true; }
    if (event.type === "keypress" && handledOption) {
      event.preventDefault();
      return false;
    }
    if (event.type !== "keydown") return true;
    handledOption = false;
    const key = event.key ?? "";
    if (!event.altKey || event.ctrlKey || event.metaKey || event.isComposing || event.keyCode === 229
      || [...key].length !== 1 || key.codePointAt(0) < 32 || key.codePointAt(0) === 127) return true;
    handledOption = true;
    event.preventDefault();
    terminal.input(key, true);
    return false;
  });
}
