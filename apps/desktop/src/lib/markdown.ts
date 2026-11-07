// Google Chat message markdown → small AST. Pure, so it is testable without React.
// Supports *bold* / **bold**, _italic_, ~strike~ / ~~strike~~, `code`, ```fenced```,
// > quotes, - bullets, 1. numbered lists, bare URLs, <url|label>, [label](url), <users/ID> and <users/all> mentions.

export type Inline =
  | { t: "text"; v: string }
  | { t: "bold"; c: Inline[] }
  | { t: "italic"; c: Inline[] }
  | { t: "strike"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "link"; href: string; label: string }
  | { t: "mention"; user: string } // "users/123" or "users/all"
  | { t: "br" };

export type Block =
  | { t: "p"; c: Inline[] }
  | { t: "code"; lang?: string; v: string }
  | { t: "quote"; c: Inline[] }
  | { t: "list"; items: Inline[][]; ordered?: boolean; start?: number };

const URL_RE = /^https?:\/\/[^\s<>]+/i;
const MENTION_RE = /^<(users\/[A-Za-z0-9_.-]+|users\/all)>/;
// What Google's formattedText really emits for a mention (the docs say <users/{id}>; the wire says this).
const CHAT_USER_RE = /^<chat-user\b[^>]*\bdata-user="(users\/[^"]+)"[^>]*>(?:[^<]*)<\/chat-user>/i;
const CHAT_TAG_RE = /^<\/?chat-[a-z]+\b[^>]*>/i;
const ANGLE_LINK_RE = /^<(https?:\/\/[^\s|>]+)(?:\|([^>]*))?>/i;
const MD_LINK_RE = /^\[([^\]\n]+)\]\((https?:\/\/[^)\s]+)\)/i;
const BULLET_RE = /^\s*[-*•]\s+/;
const NUMBER_RE = /^\s*(\d{1,3})[.)]\s+/;

function isWordChar(ch: string | undefined): boolean {
  return !!ch && /[A-Za-z0-9]/.test(ch);
}

/** Finds the closing delimiter for an emphasis run; -1 when there is none. */
function findClose(s: string, from: number, delim: string): number {
  let i = from;
  while (i < s.length) {
    const j = s.indexOf(delim, i);
    if (j < 0) return -1;
    const before = s[j - 1];
    const after = s[j + delim.length];
    const closes = before !== undefined && !/\s/.test(before) && j > from && !(delim.length === 1 && after === delim);
    if (closes && !(delim === "_" && isWordChar(after))) return j;
    i = j + 1;
  }
  return -1;
}

export function parseInline(s: string): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const flush = () => {
    if (text) out.push({ t: "text", v: text });
    text = "";
  };
  let i = 0;
  while (i < s.length) {
    const ch = s[i]!;
    const prev = s[i - 1];
    const next = s[i + 1];

    if (ch === "\n") {
      flush();
      out.push({ t: "br" });
      i += 1;
      continue;
    }
    if (ch === "`") {
      const j = s.indexOf("`", i + 1);
      if (j > i + 1) {
        flush();
        out.push({ t: "code", v: s.slice(i + 1, j) });
        i = j + 1;
        continue;
      }
    }
    if (ch === "<") {
      const rest = s.slice(i);
      const m = MENTION_RE.exec(rest);
      if (m) {
        flush();
        out.push({ t: "mention", user: m[1]! });
        i += m[0].length;
        continue;
      }
      const cu = CHAT_USER_RE.exec(rest);
      if (cu) {
        flush();
        out.push({ t: "mention", user: cu[1]! });
        i += cu[0].length;
        continue;
      }
      const ct = CHAT_TAG_RE.exec(rest);
      if (ct) {
        // Any other <chat-…> wrapper (custom emoji and the like): the markup goes, its text stays.
        i += ct[0].length;
        continue;
      }
      const l = ANGLE_LINK_RE.exec(rest);
      if (l) {
        flush();
        out.push({ t: "link", href: l[1]!, label: l[2] || l[1]! });
        i += l[0].length;
        continue;
      }
    }
    if (ch === "[") {
      const m = MD_LINK_RE.exec(s.slice(i));
      if (m) {
        flush();
        out.push({ t: "link", href: m[2]!, label: m[1]!.replace(/\s+/g, " ").trim() });
        i += m[0].length;
        continue;
      }
    }
    if (ch === "h" && (prev === undefined || !isWordChar(prev))) {
      const m = URL_RE.exec(s.slice(i));
      if (m) {
        let url = m[0];
        while (/[.,;:!?)]$/.test(url)) url = url.slice(0, -1);
        flush();
        out.push({ t: "link", href: url, label: url });
        i += url.length;
        continue;
      }
    }
    if ((ch === "*" || ch === "_" || ch === "~") && (prev === undefined || !isWordChar(prev) || ch !== "_")) {
      const double = next === ch;
      const delim = double ? ch + ch : ch;
      const start = i + delim.length;
      const first = s[start];
      if (first !== undefined && !/\s/.test(first) && first !== ch) {
        const j = findClose(s, start, delim);
        if (j > start) {
          const inner = parseInline(s.slice(start, j));
          flush();
          if (ch === "~") out.push({ t: "strike", c: inner });
          else if (ch === "_") out.push({ t: "italic", c: inner });
          else out.push({ t: double ? "bold" : "bold", c: inner });
          i = j + delim.length;
          continue;
        }
      }
    }
    text += ch;
    i += 1;
  }
  flush();
  return out;
}

export function parseMarkdown(text: string): Block[] {
  const lines = (text ?? "").replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) blocks.push({ t: "p", c: parseInline(para.join("\n")) });
    para = [];
  };
  while (i < lines.length) {
    const line = lines[i]!;
    const fence = /^```(\w*)\s*$/.exec(line);
    if (fence) {
      flushPara();
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !/^```\s*$/.test(lines[i]!)) body.push(lines[i]!), (i += 1);
      i += 1; // closing fence (or EOF)
      blocks.push(fence[1] ? { t: "code", lang: fence[1], v: body.join("\n") } : { t: "code", v: body.join("\n") });
      continue;
    }
    if (/^>\s?/.test(line)) {
      flushPara();
      const q: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i]!)) q.push(lines[i]!.replace(/^>\s?/, "")), (i += 1);
      blocks.push({ t: "quote", c: parseInline(q.join("\n")) });
      continue;
    }
    if (BULLET_RE.test(line)) {
      flushPara();
      const items: Inline[][] = [];
      while (i < lines.length && BULLET_RE.test(lines[i]!)) items.push(parseInline(lines[i]!.replace(BULLET_RE, ""))), (i += 1);
      blocks.push({ t: "list", items });
      continue;
    }
    const num = NUMBER_RE.exec(line);
    if (num) {
      flushPara();
      const items: Inline[][] = [];
      // A run keeps going while each line continues the count, so "1. a / 2. b" is one list and a stray "2024." is not.
      let expect = Number(num[1]);
      while (i < lines.length) {
        const m = NUMBER_RE.exec(lines[i]!);
        if (!m || Number(m[1]) !== expect) break;
        items.push(parseInline(lines[i]!.replace(NUMBER_RE, "")));
        expect += 1;
        i += 1;
      }
      blocks.push({ t: "list", items, ordered: true, start: Number(num[1]) });
      continue;
    }
    if (line.trim() === "") {
      flushPara();
      i += 1;
      continue;
    }
    para.push(line);
    i += 1;
  }
  flushPara();
  return blocks;
}

/** Plain text preview for sidebars and search rows: mentions become @Name, markup stripped. */
export function plainText(text: string, nameOf: (user: string) => string): string {
  const walk = (nodes: Inline[]): string =>
    nodes
      .map((n) => {
        switch (n.t) {
          case "text":
            return n.v;
          case "code":
            return n.v;
          case "link":
            return n.label;
          case "mention":
            return n.user === "users/all" ? "@all" : "@" + nameOf(n.user);
          case "br":
            return " ";
          default:
            return walk(n.c);
        }
      })
      .join("");
  return parseMarkdown(text)
    .map((b) => (b.t === "code" ? b.v : b.t === "list" ? b.items.map(walk).join(", ") : walk(b.c)))
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
}

const SKIP_HOSTS = /(^|\.)(docs|drive|chat|mail|meet|calendar)\.google\.com$/i;
/** The http(s) links in a message, in order, without repeats and without Google's own apps (those have cards already). */
export function linksOf(text: string, max = 3): string[] {
  const out: string[] = [];
  const walk = (nodes: Inline[]) => {
    for (const n of nodes) {
      if (n.t === "link") {
        let host = "";
        try { host = new URL(n.href).hostname; } catch { continue; }
        if (!SKIP_HOSTS.test(host) && !out.includes(n.href)) out.push(n.href);
      } else if ("c" in n) walk(n.c);
    }
  };
  for (const b of parseMarkdown(text)) if (b.t === "p" || b.t === "quote") walk(b.c); else if (b.t === "list") b.items.forEach(walk);
  return out.slice(0, max);
}
