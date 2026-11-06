// Runs before the app bundle: picks the splash colors from the cached theme so a light theme doesn't flash dark.
try {
  const theme = JSON.parse(localStorage.getItem("appearance") || "null")?.theme;
  const light = theme === "light" || (theme === "system" && matchMedia("(prefers-color-scheme: light)").matches);
  if (light) document.documentElement.classList.add("boot-light");
} catch {
  // No cache: dark splash.
}
