const { invoke } = window.__TAURI__.core;

export const getDeploymentConfig = () => invoke("get_deployment_config");
export const validateDeploymentResource = (resourceType, definition, replacingId = null) =>
  invoke("validate_deployment_config_resource", { resourceType, definition, replacingId });
export const saveDeploymentResource = (resourceType, definition, replacingId = null) =>
  invoke("save_deployment_config_resource", { resourceType, definition, replacingId });
export const deleteDeploymentResource = (resourceType, id) =>
  invoke("delete_deployment_config_resource", { resourceType, id });
export const duplicateDeploymentResource = (resourceType, id) =>
  invoke("duplicate_deployment_config_resource", { resourceType, id });
export const restoreDeploymentResource = (resourceType, id) =>
  invoke("restore_deployment_config_resource", { resourceType, id });
export const getDeploymentConfigBundle = () => invoke("get_deployment_config_bundle");

export const getLocalAnsibleSettings = () => invoke("get_local_ansible_settings");
export const setLocalAnsibleFolder = (path) => invoke("set_local_ansible_folder", { path });
export const getLocalDeploymentInventory = (processId) => invoke("get_local_deployment_inventory", { processId });
