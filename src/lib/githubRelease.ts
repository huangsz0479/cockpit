const LATEST_RELEASE_API_URL = "https://api.github.com/repos/huangsz0479/cockpit/releases/latest";
const RELEASE_PAGE_BASE_URL = "https://github.com/huangsz0479/cockpit/releases/tag/";

interface GitHubLatestReleaseResponse {
  tag_name?: unknown;
  body?: unknown;
}

export interface GitHubReleaseInfo {
  version: string;
  notes?: string;
  url: string;
}

function versionParts(version: string) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(version.trim());
  if (!match) throw new Error(`GitHub Release 标签不是有效版本号：${version}`);
  return {
    core: [Number(match[1]), Number(match[2]), Number(match[3])],
    prerelease: match[4]?.split(".") ?? [],
  };
}

function comparePrerelease(left: string[], right: string[]) {
  if (!left.length || !right.length) return left.length === right.length ? 0 : left.length ? -1 : 1;
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const a = left[index];
    const b = right[index];
    if (a === undefined || b === undefined) return a === b ? 0 : a === undefined ? -1 : 1;
    if (a === b) continue;
    const aNumber = /^\d+$/.test(a) ? Number(a) : null;
    const bNumber = /^\d+$/.test(b) ? Number(b) : null;
    if (aNumber !== null && bNumber !== null) return aNumber - bNumber;
    if (aNumber !== null || bNumber !== null) return aNumber !== null ? -1 : 1;
    return a.localeCompare(b);
  }
  return 0;
}

export function isNewerVersion(candidate: string, current: string) {
  const left = versionParts(candidate);
  const right = versionParts(current);
  for (let index = 0; index < left.core.length; index += 1) {
    const candidatePart = left.core[index] ?? 0;
    const currentPart = right.core[index] ?? 0;
    if (candidatePart !== currentPart) return candidatePart > currentPart;
  }
  return comparePrerelease(left.prerelease, right.prerelease) > 0;
}

export async function fetchLatestGitHubRelease(request: typeof fetch = fetch): Promise<GitHubReleaseInfo> {
  const response = await request(LATEST_RELEASE_API_URL, {
    cache: "no-store",
    headers: {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (!response.ok) throw new Error(`GitHub Releases 返回 ${response.status}`);
  const payload = await response.json() as GitHubLatestReleaseResponse;
  if (typeof payload.tag_name !== "string") throw new Error("GitHub Release 缺少版本标签");
  versionParts(payload.tag_name);
  return {
    version: payload.tag_name.replace(/^v/i, ""),
    notes: typeof payload.body === "string" ? payload.body : undefined,
    url: `${RELEASE_PAGE_BASE_URL}${encodeURIComponent(payload.tag_name)}`,
  };
}
