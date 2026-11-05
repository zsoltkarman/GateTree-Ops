const { invoke } = window.__TAURI__.core;

export const getImageHub = () => invoke("get_image_hub");
export const saveImageHubFolder = input => invoke("save_image_hub_folder", { input });
export const saveImageHubRepository = input => invoke("save_image_hub_repository", { input });
export const deleteImageHubFolder = id => invoke("delete_image_hub_folder", { id });
export const deleteImageHubRepository = id => invoke("delete_image_hub_repository", { id });
export const saveImageHubProject = input => invoke("save_image_hub_project", { input });
export const deleteImageHubProject = id => invoke("delete_image_hub_project", { id });
export const syncImageHubProject = id => invoke("sync_image_hub_project", { id });
export const openImageHubReport = url => invoke("open_image_hub_report", { url });
export const refreshImageHubArtifacts = id => invoke("refresh_image_hub_artifacts", { id });
export const saveImageHubConfig = input => invoke("save_image_hub_config", { input });
export const importImageHubConfig = path => invoke("import_image_hub_config", { path });
export const saveImageHubConfigEntry = (originalName, name, value) => invoke("save_image_hub_config_entry", { originalName, name, value });
export const deleteImageHubConfigEntry = name => invoke("delete_image_hub_config_entry", { name });

export const getImageHubAnsibleImages = () => invoke("get_image_hub_ansible_images");

export const getImageHubSecurityReport = input => invoke("get_image_hub_security_report", input);
