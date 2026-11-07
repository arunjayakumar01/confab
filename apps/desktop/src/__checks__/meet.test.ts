import { describe, expect, it } from "vitest";
import { isHuddleStart } from "../lib/meet";

const uri = "https://meet.google.com/abc-defg-hij";
const rich = (huddleStatus: string, type = "HUDDLE") => ({ annotations: [{ type: "RICH_LINK", richLinkMetadata: { uri, richLinkType: "MEET_SPACE", meetSpaceLinkData: { meetingCode: "abc-defg-hij", type, huddleStatus } } }] }) as never;

describe("isHuddleStart", () => {
  it("rings for a started huddle, by annotation or Confab's text", () => {
    expect(isHuddleStart(rich("STARTED"))).toBe(true);
    expect(isHuddleStart({ text: `Started a huddle\n${uri}` })).toBe(true);
    expect(isHuddleStart({ text: `Started a huddle\n${uri} <users/all>` })).toBe(true);
  });
  it("does not ring for anything else", () => {
    expect(isHuddleStart(rich("ENDED"))).toBe(false);
    expect(isHuddleStart({ text: `Join the video call: ${uri}` })).toBe(false);
    expect(isHuddleStart({ text: "Started a huddle, no link" })).toBe(false);
  });
});
