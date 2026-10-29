// A short two-note chime for the "Sound" setting on web and the desktop shell, where the
// notification itself is sent silent so the volume and tone are the same everywhere. Best effort:
// browsers keep an AudioContext suspended until the page has had a gesture.
let context: AudioContext | null = null;

export function playAlertChime(): void {
  try {
    const Ctor = typeof window !== "undefined" ? (window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext) : undefined;
    if (!Ctor) return;
    context ??= new Ctor();
    const ctx = context;
    if (ctx.state === "suspended") void ctx.resume().catch(() => undefined);
    const start = ctx.currentTime;
    [660, 880].forEach((hz, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      const at = start + i * 0.12;
      osc.type = "sine";
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(0.12, at + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.22);
      osc.connect(gain).connect(ctx.destination);
      osc.start(at);
      osc.stop(at + 0.24);
    });
  } catch {
    // no audio available; the notification already went out
  }
}
