export function imageHubUrl(template, config = {}, repo = {}, version = "") {
  if (!template?.trim()) return null;
  const values = { repo: repo.name, image: repo.name, ocid: repo.ocid, version,
    team: config.team, region: config.region, context: config.context,
    artifactRepository: config.artifactRepository };
  let valid = true;
  const result = template.replace(/\{([^{}]+)\}/g, (_match, key) => {
    if (!Object.hasOwn(values, key) || !values[key]) { valid = false; return ""; }
    return encodeURIComponent(values[key]);
  });
  if (!valid || /[{}]/.test(result)) return null;
  try {
    const url = new URL(result);
    return ["https:", "http:"].includes(url.protocol) && !url.username && !url.password && !url.hash ? url.href : null;
  } catch { return null; }
}

export function imageHubRepositoryUrl(config = {}, repo = {}) {
  return imageHubUrl(config.repositoryUrlTemplate, config, repo)
    ?? (typeof repo.browserUrl === "string" && !/[{}]/.test(repo.browserUrl) ? imageHubUrl(repo.browserUrl) : null)
    ?? (typeof repo.url === "string" && !/[{}]/.test(repo.url) ? imageHubUrl(repo.url) : null);
}
