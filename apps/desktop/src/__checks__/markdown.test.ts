import { describe, expect, it } from "vitest";
import { linksOf, parseInline, parseMarkdown, plainText } from "../lib/markdown";

describe("markdown", () => {
  it("parses emphasis, code and strike", () => {
    expect(parseInline("a *b* _c_ ~d~ `e` **f**")).toEqual([
      { t: "text", v: "a " }, { t: "bold", c: [{ t: "text", v: "b" }] },
      { t: "text", v: " " }, { t: "italic", c: [{ t: "text", v: "c" }] },
      { t: "text", v: " " }, { t: "strike", c: [{ t: "text", v: "d" }] },
      { t: "text", v: " " }, { t: "code", v: "e" },
      { t: "text", v: " " }, { t: "bold", c: [{ t: "text", v: "f" }] },
    ]);
  });
  it("leaves snake_case and lone asterisks alone", () => {
    expect(parseInline("snake_case_name and 2 * 3")).toEqual([{ t: "text", v: "snake_case_name and 2 * 3" }]);
  });
  it("renders mentions, bare links and <url|label>", () => {
    expect(parseInline("hi <users/1002> see https://x.test/a. and <https://y.test|Y>")).toEqual([
      { t: "text", v: "hi " }, { t: "mention", user: "users/1002" },
      { t: "text", v: " see " }, { t: "link", href: "https://x.test/a", label: "https://x.test/a" },
      { t: "text", v: ". and " }, { t: "link", href: "https://y.test", label: "Y" },
    ]);
    expect(parseInline("<users/all>")).toEqual([{ t: "mention", user: "users/all" }]);
    // Real Google formattedText spells mentions as a chat-user element; other chat-* wrappers drop their tags.
    expect(parseInline('<chat-user data-user="users/1110">Priya</chat-user> see <chat-emoji data-emoji=":party:">:party:</chat-emoji>')).toEqual([
      { t: "mention", user: "users/1110" }, { t: "text", v: " see :party:" },
    ]);
  });
  it("splits blocks: fence, quote, list, paragraphs", () => {
    const blocks = parseMarkdown("one\ntwo\n\n```ts\nconst a = 1;\n```\n> q1\n> q2\n- a\n- *b*");
    expect(blocks).toEqual([
      { t: "p", c: [{ t: "text", v: "one" }, { t: "br" }, { t: "text", v: "two" }] },
      { t: "code", lang: "ts", v: "const a = 1;" },
      { t: "quote", c: [{ t: "text", v: "q1" }, { t: "br" }, { t: "text", v: "q2" }] },
      { t: "list", items: [[{ t: "text", v: "a" }], [{ t: "bold", c: [{ t: "text", v: "b" }] }]] },
    ]);
  });
  it("numbers lists from their first item and reads [label](url) links", () => {
    expect(parseMarkdown("intro\n1. one\n2. two\nmore\n3. three")).toEqual([
      { t: "p", c: [{ t: "text", v: "intro" }] },
      { t: "list", items: [[{ t: "text", v: "one" }], [{ t: "text", v: "two" }]], ordered: true, start: 1 },
      { t: "p", c: [{ t: "text", v: "more" }] },
      { t: "list", items: [[{ t: "text", v: "three" }]], ordered: true, start: 3 },
    ]);
    expect(parseInline("see [the doc](https://docs.google.com/document/d/1/edit) now")).toEqual([
      { t: "text", v: "see " }, { t: "link", href: "https://docs.google.com/document/d/1/edit", label: "the doc" }, { t: "text", v: " now" },
    ]);
    expect(parseInline("[not a link] (x)")).toEqual([{ t: "text", v: "[not a link] (x)" }]);
  });
  it("lists a message's links once each, skipping Google's own apps", () => {
    expect(linksOf("see https://a.test/x and [doc](https://docs.google.com/document/d/1) then https://a.test/x again, <https://b.test|B>\n- https://c.test\n> https://d.test")).toEqual([
      "https://a.test/x", "https://b.test", "https://c.test",
    ]);
    expect(linksOf("no links, `https://in.code/` does not count")).toEqual([]);
  });
  it("does not choke on an unclosed fence", () => {
    expect(parseMarkdown("```\nx")).toEqual([{ t: "code", v: "x" }]);
  });
  it("plainText strips markup and names mentions", () => {
    expect(plainText("*hi* <users/1> `x`\nnext", (u) => (u === "users/1" ? "Priya" : u))).toBe("hi @Priya x next");
  });
});
