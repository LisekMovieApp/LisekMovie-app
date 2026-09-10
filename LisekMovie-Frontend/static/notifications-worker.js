// Notifications only: this worker does not cache authenticated pages or media.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('notificationclick', event => {
    event.notification.close();
    const path = event.notification.data?.url;
    if (typeof path !== 'string' || !/^\/media\/[0-9a-f-]{36}$/i.test(path)) return;
    const url = new URL(path, self.location.origin).href;
    event.waitUntil((async () => {
        const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
        const client = windows.find(window => new URL(window.url).origin === self.location.origin);
        if (client) { await client.navigate(url); await client.focus(); }
        else await self.clients.openWindow(url);
    })());
});
