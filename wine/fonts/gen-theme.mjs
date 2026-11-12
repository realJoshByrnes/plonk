// Generates wine/fonts/theme-dark.reg, theme-light.reg (HKCU, applied at page load) and fontsubst.reg (HKLM, baked into the prefix).
// Font: Selawik (SIL OFL, see LICENSE-Selawik.txt), a Segoe UI metric-compatible font: https://github.com/microsoft/Selawik
// Installed as selawk.ttf / selawkb.ttf in ~/wine-root/usr/local/share/wine/fonts (then wine/repack.sh).
import { writeFileSync } from "node:fs";
const here = new URL(".", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");

const rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)).join(" ");
function logfont(height, weight, face) {
  const b = Buffer.alloc(92);
  b.writeInt32LE(height, 0); b.writeInt32LE(weight, 16); b[23] = 1; b[26] = 0; b[27] = 5; // charset DEFAULT.. ClearType-ish quality
  b[23] = 1; b.write(face, 28, "utf16le");
  return "hex:" + [...b].map((x) => x.toString(16).padStart(2, "0")).join(",");
}
const F = "Segoe UI";
const metrics = {
  BorderWidth: "-15", PaddedBorderWidth: "0", CaptionHeight: String(-15 * 32), CaptionWidth: String(-15 * 46),
  SmCaptionHeight: String(-15 * 24), SmCaptionWidth: String(-15 * 24), ScrollWidth: "-255", ScrollHeight: "-255",
  MenuHeight: String(-15 * 24), MenuWidth: String(-15 * 24), IconSpacing: "-1125", IconVerticalSpacing: "-1125",
};
const fonts = {
  CaptionFont: logfont(-12, 700, F), SmCaptionFont: logfont(-12, 400, F), MenuFont: logfont(-12, 400, F),
  StatusFont: logfont(-12, 400, F), MessageFont: logfont(-12, 400, F), IconFont: logfont(-12, 400, F),
};
const themes = {
  dark: {
    Window: "#202020", WindowText: "#ffffff", WindowFrame: "#3a3a3a", ButtonFace: "#2b2b2b", ButtonText: "#ffffff",
    ButtonHilight: "#4a4a4a", ButtonLight: "#3a3a3a", ButtonShadow: "#1a1a1a", ButtonDkShadow: "#101010", ButtonAlternateFace: "#2b2b2b",
    Menu: "#2b2b2b", MenuText: "#ffffff", MenuBar: "#2b2b2b", MenuHilight: "#0067c0", Hilight: "#0067c0", HilightText: "#ffffff",
    ActiveTitle: "#202020", GradientActiveTitle: "#202020", TitleText: "#ffffff",
    InactiveTitle: "#2b2b2b", GradientInactiveTitle: "#2b2b2b", InactiveTitleText: "#9d9d9d", ActiveBorder: "#3a3a3a", InactiveBorder: "#2b2b2b",
    GrayText: "#8a8a8a", InfoWindow: "#2b2b2b", InfoText: "#ffffff", AppWorkSpace: "#1c1c1c", ScrollBar: "#2b2b2b", Background: "#101010", TitleText2: "#ffffff",
  },
  light: {
    Window: "#ffffff", WindowText: "#1b1b1b", WindowFrame: "#c8c8c8", ButtonFace: "#f3f3f3", ButtonText: "#1b1b1b",
    ButtonHilight: "#ffffff", ButtonLight: "#e5e5e5", ButtonShadow: "#a0a0a0", ButtonDkShadow: "#7a7a7a", ButtonAlternateFace: "#f3f3f3",
    Menu: "#f9f9f9", MenuText: "#1b1b1b", MenuBar: "#f3f3f3", MenuHilight: "#0067c0", Hilight: "#0067c0", HilightText: "#ffffff",
    ActiveTitle: "#f3f3f3", GradientActiveTitle: "#f3f3f3", TitleText: "#1b1b1b",
    InactiveTitle: "#fafafa", GradientInactiveTitle: "#fafafa", InactiveTitleText: "#8a8a8a", ActiveBorder: "#c8c8c8", InactiveBorder: "#e0e0e0",
    GrayText: "#8a8a8a", InfoWindow: "#f9f9f9", InfoText: "#1b1b1b", AppWorkSpace: "#e6e6e6", ScrollBar: "#f3f3f3", Background: "#d8d8d8",
  },
};
for (const [name, c] of Object.entries(themes)) {
  delete c.TitleText2;
  let s = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\Control Panel\\Colors]\r\n";
  for (const [k, v] of Object.entries(c)) s += `"${k}"="${rgb(v)}"\r\n`;
  s += "\r\n[HKEY_CURRENT_USER\\Control Panel\\Desktop\\WindowMetrics]\r\n";
  for (const [k, v] of Object.entries(metrics)) s += `"${k}"="${v}"\r\n`;
  for (const [k, v] of Object.entries(fonts)) s += `"${k}"=${v}\r\n`;
  s += "\r\n[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\ThemeManager]\r\n\"ThemeActive\"=\"0\"\r\n"; // classic controls follow the colours above
  writeFileSync(here + `theme-${name}.reg`, s);
}
let s = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows NT\\CurrentVersion\\FontSubstitutes]\r\n";
for (const k of ["Segoe UI", "MS Shell Dlg", "MS Shell Dlg 2", "Tahoma", "Microsoft Sans Serif", "MS Sans Serif", "Arial", "Helvetica"]) s += `"${k}"="Selawik"\r\n`;
writeFileSync(here + "fontsubst.reg", s);
