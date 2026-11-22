import { describe, expect, it } from "vitest";
import { isInside } from "../paths";

describe("isInside", () => {
  it("counts the folder itself and what is below it", () => {
    expect(isInside("/Trip", "/Trip")).toBe(true);
    expect(isInside("/Trip/Day 1", "/Trip")).toBe(true);
    expect(isInside("/Trip/Day 1", "/Trip/")).toBe(true);
  });

  it("doesn't confuse folders that start with the same letters", () => {
    expect(isInside("/Trip 2", "/Trip")).toBe(false);
    expect(isInside("/Trip 2/a", "/Trip")).toBe(false);
  });

  it("understands Windows paths", () => {
    expect(isInside("D:\\Photos\\2024", "D:\\Photos")).toBe(true);
    expect(isInside("D:\\Photos 2024", "D:\\Photos")).toBe(false);
  });
});
