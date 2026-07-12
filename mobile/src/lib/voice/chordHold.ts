// Desktop push-to-talk state shared across the Composer→VoiceRecordingPill swap: the chord
// keydown that starts a recording (Composer.tsx) unmounts the composer's input row and mounts
// the pill, so the keyup that ends a hold only ever arrives while the pill is mounted. Module
// level (not React state or a prop) because both sides need the same timestamp across that
// mount boundary. Only ever touched behind Platform.OS === "web" guards.
export const PUSH_TO_TALK_MIN_MS = 400;

// non-null ⇔ the chord that started the current recording has not released yet
export const chordHold: { startedAt: number | null } = { startedAt: null };
