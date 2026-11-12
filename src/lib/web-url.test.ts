import { describe, expect, it } from "vitest";
import { webUrl } from "./web-url";

describe("webUrl", () => {
  it("accepts_http_and_https_pages", () => {
    expect(webUrl("https://a.test/x?y=1")).toBe("https://a.test/x?y=1");
    expect(webUrl("http://a.test")).toBe("http://a.test/");
  });

  it("reads_a_scheme_less_host_as_https_like_the_matcher", () => {
    expect(webUrl("example.com/login")).toBe("https://example.com/login");
  });

  it("rejects_every_other_scheme_and_non_urls", () => {
    for (const raw of ["javascript:alert(1)", "file:///etc/passwd", "ftp://a.test", "data:text/html,x", "mailto:a@b.test", "", "  ", "not a url", "localhost:8080"]) {
      expect(webUrl(raw), raw).toBeNull();
    }
  });
});
