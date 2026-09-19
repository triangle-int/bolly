/** Refresh scoped media URLs without retaining a control token or a stale grant.
 * @param {() => Promise<{url: string, refresh_after_seconds: number}>} issue
 * @param {(url: string) => void} apply
 * @param {{schedule?: typeof setTimeout, cancel?: typeof clearTimeout}} timers
 * @returns {(() => void) & {refreshNow: () => void}}
 */
export function maintainResource(issue, apply, timers = {}) {
    const schedule = timers.schedule ?? setTimeout;
    const cancel = timers.cancel ?? clearTimeout;
    let stopped = false;
    let inFlight = false;
    let immediateBudget = 2;
    /** @type {ReturnType<typeof setTimeout> | undefined} */
    let timer;
    async function refresh() {
        if (stopped || inFlight) return;
        inFlight = true;
        let delay = 600000;
        try {
            const grant = await issue();
            const seconds = Number(grant.refresh_after_seconds);
            delay = Number.isFinite(seconds) ? Math.min(3600, Math.max(1, seconds)) * 1000 : delay;
            if (!stopped) apply(grant.url);
        } catch {
            if (!stopped) apply('');
            delay = 30000;
        }
        inFlight = false;
        if (!stopped) timer = schedule(refresh, delay);
    }
    function refreshNow() {
        if (stopped || inFlight || immediateBudget === 0) return;
        immediateBudget -= 1;
        if (timer !== undefined) cancel(timer);
        timer = undefined;
        void refresh();
    }
    void refresh();
    /** @type {(() => void) & {refreshNow: () => void}} */
    const stop = Object.assign(
        () => { stopped = true; if (timer !== undefined) cancel(timer); },
        { refreshNow },
    );
    return stop;
}
