// Huddles: in-app audio between Confab clients. A full mesh of RTCPeerConnections, one per other member, signalled
// over the relay (huddle_signal carries SDP and ICE). The member whose name sorts lower makes the offer, so both
// sides of every pair agree on who starts without a round trip. Google Meet stays the fallback for everyone else.

export interface HuddleSignalPayload { kind: "offer" | "answer" | "ice"; sdp?: string; candidate?: RTCIceCandidateInit | null }

export interface HuddleHooks {
  me: string;
  send: (space: string, to: string, payload: HuddleSignalPayload) => void;
  onPeers: (peers: Record<string, "connecting" | "connected" | "failed">) => void;
  onSpeaking: (users: string[]) => void;
}

const ICE: RTCIceServer[] = [{ urls: ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302"] }];
const LEVEL_MS = 180;
const SPEAK_AT = 0.035;

export class Huddle {
  readonly space: string;
  private readonly hooks: HuddleHooks;
  private readonly pcs = new Map<string, RTCPeerConnection>();
  private readonly state: Record<string, "connecting" | "connected" | "failed"> = {};
  private readonly audio = new Map<string, HTMLAudioElement>();
  private readonly analysers = new Map<string, AnalyserNode>();
  private readonly pendingIce = new Map<string, RTCIceCandidateInit[]>();
  private stream: MediaStream | null = null;
  private ctx: AudioContext | null = null;
  private levelTimer = 0;
  private closed = false;
  /** True when the microphone could not be opened: the member hears others but sends nothing. */
  listenOnly = false;

  constructor(space: string, hooks: HuddleHooks) {
    this.space = space;
    this.hooks = hooks;
  }

  async open(): Promise<void> {
    try {
      this.stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
      this.watch(this.hooks.me, this.stream);
    } catch (e) {
      console.warn("huddle: no microphone", e);
      this.listenOnly = true;
    }
    this.levelTimer = window.setInterval(() => this.tick(), LEVEL_MS);
  }

  get muted(): boolean {
    return !!this.stream && this.stream.getAudioTracks().every((t) => !t.enabled);
  }
  setMuted(on: boolean): void {
    this.stream?.getAudioTracks().forEach((t) => { t.enabled = !on; });
  }

  /** The relay said who is in the huddle: connect to newcomers, drop the gone. */
  members(users: string[]): void {
    if (this.closed) return;
    const others = users.filter((u) => u !== this.hooks.me);
    for (const u of others) if (!this.pcs.has(u)) this.connect(u, this.hooks.me < u);
    for (const u of [...this.pcs.keys()]) if (!others.includes(u)) this.drop(u);
    this.hooks.onPeers({ ...this.state });
  }

  /** SDP or ICE from a peer. An offer from someone we have no connection with yet creates one (they sort lower). */
  async signal(from: string, p: HuddleSignalPayload): Promise<void> {
    if (this.closed) return;
    let pc = this.pcs.get(from);
    if (!pc) pc = this.connect(from, false);
    try {
      if (p.kind === "offer" && p.sdp) {
        await pc.setRemoteDescription({ type: "offer", sdp: p.sdp });
        await this.flushIce(from, pc);
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        this.hooks.send(this.space, from, { kind: "answer", sdp: answer.sdp });
      } else if (p.kind === "answer" && p.sdp) {
        if (pc.signalingState === "have-local-offer") {
          await pc.setRemoteDescription({ type: "answer", sdp: p.sdp });
          await this.flushIce(from, pc);
        }
      } else if (p.kind === "ice" && p.candidate) {
        if (pc.remoteDescription) await pc.addIceCandidate(p.candidate);
        else (this.pendingIce.get(from) ?? this.pendingIce.set(from, []).get(from)!).push(p.candidate);
      }
    } catch (e) {
      console.warn("huddle signal", from, p.kind, e);
    }
  }

  close(): void {
    this.closed = true;
    window.clearInterval(this.levelTimer);
    for (const u of [...this.pcs.keys()]) this.drop(u);
    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
    void this.ctx?.close();
    this.ctx = null;
  }

  private connect(user: string, offer: boolean): RTCPeerConnection {
    const pc = new RTCPeerConnection({ iceServers: ICE });
    this.pcs.set(user, pc);
    this.state[user] = "connecting";
    if (this.stream) for (const t of this.stream.getAudioTracks()) pc.addTrack(t, this.stream);
    else pc.addTransceiver("audio", { direction: "recvonly" });
    pc.onicecandidate = (e) => this.hooks.send(this.space, user, { kind: "ice", candidate: e.candidate ? e.candidate.toJSON() : null });
    pc.ontrack = (e) => {
      const stream = e.streams[0] ?? new MediaStream([e.track]);
      let el = this.audio.get(user);
      if (!el) {
        el = document.createElement("audio");
        el.autoplay = true;
        el.dataset.huddle = user;
        document.body.appendChild(el);
        this.audio.set(user, el);
      }
      el.srcObject = stream;
      void el.play().catch(() => {});
      this.watch(user, stream);
    };
    pc.onconnectionstatechange = () => {
      const s = pc.connectionState;
      this.state[user] = s === "connected" ? "connected" : s === "failed" || s === "disconnected" || s === "closed" ? "failed" : "connecting";
      this.hooks.onPeers({ ...this.state });
    };
    if (offer) {
      void pc.createOffer().then(async (o) => {
        await pc.setLocalDescription(o);
        this.hooks.send(this.space, user, { kind: "offer", sdp: o.sdp });
      }).catch((e) => console.warn("huddle offer", e));
    }
    return pc;
  }

  private async flushIce(user: string, pc: RTCPeerConnection): Promise<void> {
    const list = this.pendingIce.get(user) ?? [];
    this.pendingIce.delete(user);
    for (const c of list) await pc.addIceCandidate(c).catch(() => {});
  }

  private drop(user: string): void {
    this.pcs.get(user)?.close();
    this.pcs.delete(user);
    delete this.state[user];
    this.analysers.delete(user);
    this.pendingIce.delete(user);
    const el = this.audio.get(user);
    if (el) { el.srcObject = null; el.remove(); }
    this.audio.delete(user);
  }

  /** A tiny analyser per stream feeds the speaking rings; nothing is recorded. */
  private watch(user: string, stream: MediaStream): void {
    try {
      this.ctx ??= new AudioContext();
      const src = this.ctx.createMediaStreamSource(stream);
      const an = this.ctx.createAnalyser();
      an.fftSize = 512;
      src.connect(an);
      this.analysers.set(user, an);
    } catch (e) {
      console.warn("huddle analyser", e);
    }
  }

  private tick(): void {
    const speaking: string[] = [];
    const buf = new Uint8Array(256);
    for (const [user, an] of this.analysers) {
      if (user === this.hooks.me && this.muted) continue;
      an.getByteTimeDomainData(buf);
      let sum = 0;
      for (const v of buf) { const d = (v - 128) / 128; sum += d * d; }
      if (Math.sqrt(sum / buf.length) > SPEAK_AT) speaking.push(user);
    }
    this.hooks.onSpeaking(speaking);
  }
}
