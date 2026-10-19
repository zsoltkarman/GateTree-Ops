const { invoke } = window.__TAURI__.core;
export const listQuickLaunchItems = () => invoke("list_quick_launch_items");
const change = async (command, args) => {
  const items = await invoke(command, args);
  window.dispatchEvent(new Event("gatetree-ops:quick-launch-changed"));
  return items;
};
export const pinQuickLaunchItem = (id) => change("pin_quick_launch_item", { id });
export const renameQuickLaunchItem = (id, label) => change("rename_quick_launch_item", { id, label });
export const removeQuickLaunchItem = (id) => change("remove_quick_launch_item", { id });
export const reorderQuickLaunchItems = (ids) => change("reorder_quick_launch_items", { ids });
