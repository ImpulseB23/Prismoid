// Username color normalization for chat. Twitch (and YouTube/Kick) lets
// users pick any hex color; many of those are unreadable on the app's
// dark background (e.g. #0000FF, #2E0000). We boost luminance until the
// color clears a minimum readable threshold against #0e0e10, matching
// the behavior the official Twitch web client uses for "Adjust Colors".

const FALLBACK = "#9147ff";

// WCAG-style relative luminance threshold. 0.20 keeps deep colors moody
// but readable; anything lower (#000080 etc.) gets lifted toward white.
const MIN_LUMINANCE = 0.2;

interface Rgb {
  r: number;
  g: number;
  b: number;
}

function parseHex(input: string): Rgb | null {
  let s = input.trim();
  if (s.startsWith("#")) s = s.slice(1);
  if (s.length === 3) {
    s = s
      .split("")
      .map((c) => c + c)
      .join("");
  }
  if (s.length !== 6 || !/^[0-9a-fA-F]{6}$/.test(s)) return null;
  return {
    r: parseInt(s.slice(0, 2), 16),
    g: parseInt(s.slice(2, 4), 16),
    b: parseInt(s.slice(4, 6), 16),
  };
}

function srgbToLinear(c: number): number {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}

function relativeLuminance({ r, g, b }: Rgb): number {
  return (
    0.2126 * srgbToLinear(r) +
    0.7152 * srgbToLinear(g) +
    0.0722 * srgbToLinear(b)
  );
}

function toHex({ r, g, b }: Rgb): string {
  const h = (n: number) => Math.round(n).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}`;
}

// Iteratively lift each channel toward white until the color clears the
// minimum luminance. A pure ratio scale would still produce black for
// black inputs, so we add a constant floor before scaling.
function liftToReadable(rgb: Rgb): Rgb {
  let { r, g, b } = rgb;
  // Seed pitch-black with a small grey so the loop has something to scale.
  if (r + g + b < 24) {
    r = g = b = 24;
  }
  for (let i = 0; i < 16; i++) {
    if (relativeLuminance({ r, g, b }) >= MIN_LUMINANCE) break;
    r = Math.min(255, r + (255 - r) * 0.2 + 8);
    g = Math.min(255, g + (255 - g) * 0.2 + 8);
    b = Math.min(255, b + (255 - b) * 0.2 + 8);
  }
  return { r, g, b };
}

export function normalizeUserColor(color: string | null | undefined): string {
  if (!color) return FALLBACK;
  const rgb = parseHex(color);
  if (!rgb) return FALLBACK;
  if (relativeLuminance(rgb) >= MIN_LUMINANCE) return toHex(rgb);
  return toHex(liftToReadable(rgb));
}

// Format a Unix-millis timestamp as a short 24-hour HH:MM string in the
// user's local timezone. Stable across renders for the same input so the
// virtual scroller doesn't churn.
export function formatTimestamp(unixMillis: number): string {
  const d = new Date(unixMillis);
  if (Number.isNaN(d.getTime())) return "";
  const hh = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  return `${hh}:${mm}`;
}
