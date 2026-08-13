import { describe, expect, it, vi } from "vitest";
import { fetchLatestGitHubRelease, isNewerVersion } from "./githubRelease";

describe("GitHub release updates", () => {
  it("compares stable and prerelease semantic versions", () => {
    expect(isNewerVersion("v1.2.0", "1.1.9")).toBe(true);
    expect(isNewerVersion("1.2.0", "1.2.0")).toBe(false);
    expect(isNewerVersion("1.2.0-beta.2", "1.2.0-beta.1")).toBe(true);
    expect(isNewerVersion("1.2.0-beta.1", "1.2.0")).toBe(false);
  });

  it("loads the latest public release and builds a trusted GitHub page URL", async () => {
    const request = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: vi.fn().mockResolvedValue({ tag_name: "v1.4.0", body: "Release notes" }),
    });

    await expect(fetchLatestGitHubRelease(request)).resolves.toEqual({
      version: "1.4.0",
      notes: "Release notes",
      url: "https://github.com/huangsz0479/cockpit/releases/tag/v1.4.0",
    });
    expect(request).toHaveBeenCalledWith(
      "https://api.github.com/repos/huangsz0479/cockpit/releases/latest",
      expect.objectContaining({ cache: "no-store" }),
    );
  });

  it("rejects invalid release responses", async () => {
    const request = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: vi.fn().mockResolvedValue({ tag_name: "latest" }),
    });
    await expect(fetchLatestGitHubRelease(request)).rejects.toThrow("有效版本号");
  });
});
