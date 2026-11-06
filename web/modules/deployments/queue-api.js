const { invoke } = window.__TAURI__.core;

export const getDeploymentQueue = ({ includeLogs = true } = {}) => invoke("get_deployment_queue", { includeLogs });
export const getDeploymentQueueLog = ({ planId, jobId = null, offset = 0, hash = null }) => invoke("get_deployment_queue_log", { planId, jobId, offset, hash });
export const createDeploymentPlan = (definition) => invoke("create_deployment_plan", { definition });
export const updateDeploymentPlan = (id, definition) => invoke("update_deployment_plan", { id, definition });
export const deleteDeploymentPlan = (id) => invoke("delete_deployment_plan", { id });
export const archiveDeploymentPlan = (id) => invoke("archive_deployment_plan", { id });
export const controlDeploymentQueueItem = (itemType, id, action) =>
  invoke("control_deployment_queue_item", { itemType, id, action });

export const getCompletedPlanCopy = (id) => invoke("get_completed_deployment_plan_copy", { id });

export const createDeploymentPlanBatch = (groupId, groupName, definitions) => invoke("create_deployment_plan_batch", { groupId, groupName, definitions });
