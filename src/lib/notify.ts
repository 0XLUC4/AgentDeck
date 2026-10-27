import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { pushNotify } from "./ipc";

let permission: Promise<boolean> | undefined;
let audio: AudioContext | undefined;

const canNotify = () =>
  (permission ??= (async () => (await isPermissionGranted()) || (await requestPermission()) === "granted")());

/** Short two-tone beep generated with WebAudio (no asset). */
function beep(): void {
  audio ??= new AudioContext();
  const osc = audio.createOscillator();
  const gain = audio.createGain();
  osc.frequency.setValueAtTime(880, audio.currentTime);
  osc.frequency.setValueAtTime(660, audio.currentTime + 0.09);
  gain.gain.setValueAtTime(0.15, audio.currentTime);
  gain.gain.exponentialRampToValueAtTime(0.001, audio.currentTime + 0.25);
  osc.connect(gain).connect(audio.destination);
  osc.start();
  osc.stop(audio.currentTime + 0.25);
}

/** Desktop notification, beep, and a push to `phoneUrl` (ntfy topic or webhook) when set. */
export async function notify(
  title: string,
  body: string,
  { system = true, sound = true, phoneUrl = "" } = {},
): Promise<void> {
  if (phoneUrl) pushNotify(phoneUrl, title, body).catch((err) => console.warn("phone push failed", err));
  try {
    if (sound) beep();
    if (!system) return;
    if (await canNotify()) sendNotification({ title, body });
  } catch (err) {
    console.warn("notification failed", err);
  }
}
