/**
 * Web Audio API notification chimes for DIT Pro.
 *
 * Four distinct audio patterns for different events,
 * with per-event enable/disable and volume control.
 */

export type ChimeEvent = "taskComplete" | "taskFailed" | "sourceReleased" | "warning";

export interface SoundSettings {
  enabled: boolean;
  taskComplete: boolean;
  taskFailed: boolean;
  sourceReleased: boolean;
  warning: boolean;
  volume: number; // 0.0–1.0
}

export const DEFAULT_SOUND_SETTINGS: SoundSettings = {
  enabled: true,
  taskComplete: true,
  taskFailed: true,
  sourceReleased: true,
  warning: true,
  volume: 0.5,
};

interface Note {
  freq: number;
  time: number;
}

// Chime definitions: frequency sequences and durations
const CHIME_PATTERNS: Record<ChimeEvent, { notes: Note[]; duration: number }> = {
  // Ascending major chord: C5–E5–G5–C6
  taskComplete: {
    notes: [
      { freq: 523, time: 0 },
      { freq: 659, time: 0.15 },
      { freq: 784, time: 0.3 },
      { freq: 1047, time: 0.45 },
    ],
    duration: 0.6,
  },
  // Descending minor: E5–C5–Ab4
  taskFailed: {
    notes: [
      { freq: 659, time: 0 },
      { freq: 523, time: 0.25 },
      { freq: 415, time: 0.5 },
    ],
    duration: 0.8,
  },
  // Existing pattern: A5–D6–A6
  sourceReleased: {
    notes: [
      { freq: 880, time: 0 },
      { freq: 1175, time: 0.12 },
      { freq: 1760, time: 0.24 },
    ],
    duration: 0.5,
  },
  // Short alert: single A5
  warning: {
    notes: [{ freq: 880, time: 0 }],
    duration: 0.2,
  },
};

/**
 * Play a notification chime if the event is enabled in settings.
 */
export function playChime(event: ChimeEvent, settings: SoundSettings): void {
  if (!settings.enabled || !settings[event]) return;

  try {
    const ctx = new AudioContext();
    const pattern = CHIME_PATTERNS[event];
    const vol = Math.max(0, Math.min(1, settings.volume));

    for (const note of pattern.notes) {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.connect(gain);
      gain.connect(ctx.destination);

      osc.frequency.setValueAtTime(note.freq, ctx.currentTime + note.time);
      gain.gain.setValueAtTime(vol * 0.3, ctx.currentTime + note.time);
      gain.gain.exponentialRampToValueAtTime(
        0.01,
        ctx.currentTime + note.time + pattern.duration * 0.8
      );

      osc.start(ctx.currentTime + note.time);
      osc.stop(ctx.currentTime + pattern.duration);
    }
  } catch {
    /* Audio not available — skip silently */
  }
}
