// Makes the page installable and able to open offline. Only the shell is cached: status and
// commands must never be served from a cache.

const SHELL = "toglet-remote-v2";
const FILES = ["./", "./index.html", "./app.js", "./icon.svg", "./manifest.webmanifest"];

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(SHELL).then((cache) => cache.addAll(FILES)));
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(names.filter((name) => name !== SHELL).map((name) => caches.delete(name))),
      ),
  );
  self.clients.claim();
});

// Network-first, so a redeploy reaches an installed page on its next load; the cache name never
// changes, so cache-first would serve the old page until site data is cleared.
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  const isShell =
    event.request.method === "GET" &&
    FILES.some((file) => url.pathname.endsWith(file.replace("./", "")));
  if (!isShell) return;
  event.respondWith(
    fetch(event.request)
      .then((fresh) => {
        const copy = fresh.clone();
        event.waitUntil(caches.open(SHELL).then((cache) => cache.put(event.request, copy)));
        return fresh;
      })
      // Offline: the only time the cache is read.
      .catch(() => caches.match(event.request).then((hit) => hit ?? Response.error())),
  );
});
