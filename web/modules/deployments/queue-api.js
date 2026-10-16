const { invoke } = window.__TAURI__.core;

export const getDeploymentQueue = () => invoke("get_deployment_queue");
export const createDeploymentPlan = (definition) => invoke("create_deployment_plan", { definition });
export const updateDeploymentPlan = (id, definition) => invoke("update_deployment_plan", { id, definition });
export const deleteDeploymentPlan = (id) => invoke("delete_deployment_plan", { id });
export const controlDeploymentQueueItem = (itemType, id, action) =>
  invoke("control_deployment_queue_item", { itemType, id, action });
