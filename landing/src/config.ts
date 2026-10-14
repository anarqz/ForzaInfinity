// Build-time settings (landing/.env, see .env.example).
const env = import.meta.env;

export const REPO = (env.VITE_REPO as string | undefined) || "anarqz/ForzaInfinity";
export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASE = `${REPO_URL}/releases/latest/download`;
export const DOWNLOAD_ZIP = `${RELEASE}/ForzaInfinity.zip`;
export const INSTALL_SH = `curl -fsSL ${RELEASE}/install.sh | bash`;
export const INSTALL_PS1 = `irm ${RELEASE}/install.ps1 | iex`;
export const STATS_URL = (env.VITE_STATS_URL as string | undefined) || "http://localhost:8081/infinity/v1/stats";
export const SERVER_ADDRESS = (env.VITE_SERVER_ADDRESS as string | undefined) || "";
