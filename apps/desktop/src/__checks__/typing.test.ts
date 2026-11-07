import { describe, expect, it } from "vitest";
import { typingText } from "../lib/typing";

describe("typingText", () => {
  it("matches the typing strip wording", () => {
    expect(typingText([])).toBeNull();
    expect(typingText(["Priya"])).toBe("Priya is typing…");
    expect(typingText(["Priya", "Rahul"])).toBe("Priya and Rahul are typing…");
    expect(typingText(["Priya", "Rahul", "Sana"])).toBe("Several people are typing…");
    expect(typingText(["", "Priya"])).toBe("Priya is typing…");
  });
});
