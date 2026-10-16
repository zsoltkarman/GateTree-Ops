const { invoke } = window.__TAURI__.core;

export const getAiSkills = () => invoke("get_ai_skills");
export const createAiSkillLibrary = (name, path) => invoke("create_ai_skill_library", { name, path });
export const updateAiSkillLibrary = (id, name, path) => invoke("update_ai_skill_library", { id, name, path });
export const deleteAiSkillLibrary = (id) => invoke("delete_ai_skill_library", { id });
export const refreshAiSkills = () => invoke("refresh_ai_skills");
export const readAiSkill = (id) => invoke("read_ai_skill", { id });
export const runAiSkill = (id, input) => invoke("run_ai_skill", { id, input });
