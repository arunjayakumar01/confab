// A compact built-in set. note: ~80 emoji with search; no skin tones, no custom emoji, no recents.
export interface EmojiEntry { u: string; n: string }
export const EMOJI: EmojiEntry[] = [
  { u: "👍", n: "thumbs up +1 yes" }, { u: "👎", n: "thumbs down -1 no" }, { u: "❤️", n: "heart love" },
  { u: "😂", n: "joy laugh lol" }, { u: "😄", n: "smile happy" }, { u: "😊", n: "blush smile" },
  { u: "😉", n: "wink" }, { u: "😍", n: "heart eyes love" }, { u: "🤔", n: "thinking hmm" },
  { u: "😅", n: "sweat smile" }, { u: "😢", n: "cry sad" }, { u: "😭", n: "sob crying" },
  { u: "😡", n: "angry rage" }, { u: "😱", n: "scream shock" }, { u: "🙄", n: "eye roll" },
  { u: "😴", n: "sleep tired" }, { u: "🤯", n: "mind blown" }, { u: "🥳", n: "party celebrate" },
  { u: "🤗", n: "hug" }, { u: "🤝", n: "handshake deal" }, { u: "🙏", n: "pray thanks please" },
  { u: "👏", n: "clap applause" }, { u: "🙌", n: "raised hands hooray" }, { u: "👀", n: "eyes looking" },
  { u: "👋", n: "wave hello bye" }, { u: "✅", n: "check done yes" }, { u: "❌", n: "cross no x" },
  { u: "⭐", n: "star" }, { u: "🔥", n: "fire hot lit" }, { u: "💯", n: "100 hundred" },
  { u: "🎉", n: "tada party" }, { u: "🚀", n: "rocket ship launch" }, { u: "💡", n: "idea bulb" },
  { u: "⚡", n: "zap lightning" }, { u: "🐛", n: "bug" }, { u: "🔧", n: "wrench fix" },
  { u: "🛠️", n: "tools hammer" }, { u: "📌", n: "pin pushpin" }, { u: "📎", n: "paperclip" },
  { u: "📝", n: "memo note" }, { u: "📣", n: "megaphone announce" }, { u: "🔔", n: "bell notify" },
  { u: "🔕", n: "bell off mute" }, { u: "🔒", n: "lock" }, { u: "🔑", n: "key" },
  { u: "⏰", n: "alarm clock" }, { u: "📅", n: "calendar date" }, { u: "☕", n: "coffee" },
  { u: "🍕", n: "pizza" }, { u: "🍰", n: "cake" }, { u: "🍺", n: "beer" },
  { u: "🌮", n: "taco" }, { u: "🍎", n: "apple" }, { u: "🌧️", n: "rain cloud" },
  { u: "☀️", n: "sun sunny" }, { u: "🌙", n: "moon night" }, { u: "🌈", n: "rainbow" },
  { u: "🌱", n: "seedling plant" }, { u: "🐶", n: "dog" }, { u: "🐱", n: "cat" },
  { u: "🦄", n: "unicorn" }, { u: "🐢", n: "turtle slow" }, { u: "🏃", n: "running run" },
  { u: "🧠", n: "brain" }, { u: "💪", n: "muscle strong" }, { u: "🫡", n: "salute" },
  { u: "🤷", n: "shrug" }, { u: "🫠", n: "melting" }, { u: "💀", n: "skull dead" },
  { u: "👻", n: "ghost" }, { u: "🎯", n: "dart target" }, { u: "🏆", n: "trophy win" },
  { u: "🎨", n: "art palette design" }, { u: "🎵", n: "music note" }, { u: "📷", n: "camera" },
  { u: "💻", n: "laptop computer" }, { u: "📱", n: "phone mobile" }, { u: "🖥️", n: "desktop" },
  { u: "🧪", n: "test tube experiment" }, { u: "🚧", n: "construction wip" }, { u: "🔍", n: "search magnifier" },
  { u: "💬", n: "speech bubble comment" }, { u: "🙈", n: "see no evil monkey" }, { u: "🤞", n: "fingers crossed" },
];
export function searchEmoji(q: string): EmojiEntry[] {
  const s = q.trim().toLowerCase();
  if (!s) return EMOJI;
  return EMOJI.filter((e) => e.n.includes(s) || e.u === s);
}
