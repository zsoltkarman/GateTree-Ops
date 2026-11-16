const { invoke } = window.__TAURI__.core;

export const getDeploymentTasks = () => invoke("get_deployment_tasks");
export const saveDeploymentTask = task => invoke("save_deployment_task", { task });
export const deleteDeploymentTask = (id, revision) => invoke("delete_deployment_task", { id, revision });
export const linkDeploymentTaskPlan = (itemId, planId) => invoke("link_deployment_task_plan", { itemId, planId });
