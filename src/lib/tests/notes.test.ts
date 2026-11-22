import { describe, expect, it } from "vitest";
import { plainNotes } from "../notes";

describe("plainNotes", () => {
  it("shows release notes without Markdown marks", () => {
    const md = "## What's new\r\n\r\n- **Safer** sharing\n* See [the guide](https://example.com)\n\n\n\nRun `npm test`";
    expect(plainNotes(md)).toBe("What's new\n\n• Safer sharing\n• See the guide\n\nRun npm test");
  });

  it("works the same every time (no leftover regex state)", () => {
    expect(plainNotes("**a**")).toBe("a");
    expect(plainNotes("**a**")).toBe("a");
  });
});
