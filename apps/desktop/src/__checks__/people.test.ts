import { describe, expect, it } from "vitest";
import { personToUser } from "../lib/people";

describe("personToUser", () => {
  it("maps a directory person to a Chat user", () => {
    expect(personToUser({
      resourceName: "people/1002",
      names: [{ displayName: "Priya Nair" }],
      emailAddresses: [{ value: "priya@example.test" }],
      photos: [{ url: "https://img.test/priya.png" }],
    })).toEqual({ name: "users/1002", displayName: "Priya Nair", email: "priya@example.test", avatarUrl: "https://img.test/priya.png", type: "HUMAN" });
    expect(personToUser({ resourceName: "people/9" })).toEqual({ name: "users/9", type: "HUMAN" });
  });
});
