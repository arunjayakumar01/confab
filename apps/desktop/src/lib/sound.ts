// A short two-note chime for messages that arrive out of view, synthesised so there is no asset to ship.
// WebKit only starts audio after a user gesture; the first click or key unlocks the context.
let ctx: AudioContext | null = null;
let unlocked = false;

export function unlockAudio(): void {
  if (unlocked) return;
  const once = () => {
    unlocked = true;
    ctx ??= new AudioContext();
    void ctx.resume();
    window.removeEventListener("pointerdown", once, true);
    window.removeEventListener("keydown", once, true);
  };
  window.addEventListener("pointerdown", once, true);
  window.addEventListener("keydown", once, true);
}

export function chime(): void {
  if (!unlocked || !ctx || ctx.state !== "running") return;
  const t0 = ctx.currentTime;
  for (const [freq, at] of [[659.25, 0], [880, 0.11]] as const) {
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.type = "sine";
    o.frequency.value = freq;
    g.gain.setValueAtTime(0.0001, t0 + at);
    g.gain.exponentialRampToValueAtTime(0.09, t0 + at + 0.015);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + at + 0.28);
    o.connect(g).connect(ctx.destination);
    o.start(t0 + at);
    o.stop(t0 + at + 0.3);
  }
}
