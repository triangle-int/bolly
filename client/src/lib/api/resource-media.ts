import { mediaUrl, memoryMediaUrl, type ResourceGrant } from './client.js';
import { maintainResource } from './resource-refresh.js';

export interface MediaResource { slug: string; kind: 'files' | 'memory'; path: string }
export function issueResource(resource: MediaResource): Promise<ResourceGrant> {
    return resource.kind === 'files' ? mediaUrl(resource.slug, resource.path) : memoryMediaUrl(resource.slug, resource.path);
}

export function resourceMedia(node: HTMLElement, resource: MediaResource) {
    function bind(value: MediaResource) {
        node.removeAttribute('src');
        const maintenance = maintainResource(() => issueResource(value), url => {
            if (url) node.setAttribute('src', url);
            else node.removeAttribute('src');
        });
        node.addEventListener('error', maintenance.refreshNow);
        const stop = () => {
            node.removeEventListener('error', maintenance.refreshNow);
            maintenance();
        };
        return stop;
    }
    let stop = bind(resource);
    return { update(value: MediaResource) { stop(); stop = bind(value); }, destroy() { stop(); } };
}

/** Interpret only exact local resource identities, including historical saved links. */
export function resourceFromUrl(value: string, slug?: string, expectedOrigin?: string): MediaResource | null {
    try {
        const baseOrigin = expectedOrigin
            ?? (typeof location === 'undefined' ? 'https://resource.invalid' : location.origin);
        const url = new URL(value, baseOrigin);
        if (url.origin !== baseOrigin || url.username || url.password) return null;
        const legacyFile = url.pathname.match(/^\/api\/instances\/([^/]+)\/uploads\/([^/]+)\/file$/);
        const legacyMemory = url.pathname.match(/^\/api\/instances\/([^/]+)\/memory\/(.+)$/);
        const match = url.pathname.match(/^\/resources\/(?:browser|model-provider|native-relay)\/(files|memory)\/([^/]+)\/(.+)$/)
            ?? url.pathname.match(/^\/public\/(files|memory)\/([^/]+)\/(.+)$/)
            ?? (legacyFile ? ['', 'files', legacyFile[1], legacyFile[2]] : null)
            ?? (legacyMemory ? ['', 'memory', legacyMemory[1], legacyMemory[2]] : null);
        if (!match) return null;
        const components = match[3].split('/').map(decodeURIComponent);
        const encode = (component: string) => encodeURIComponent(component).replace(/[!'()*]/g, char => `%${char.charCodeAt(0).toString(16).toUpperCase()}`);
        const resourceSlug = decodeURIComponent(match[2]);
        if (!/^[A-Za-z0-9_-]+$/.test(resourceSlug)
            || resourceSlug !== match[2]
            || components.some(part => !part || part === '.' || part === '..' || part.includes('/') || part.includes('\\') || part.normalize('NFC') !== part)
            || components.map(encode).join('/') !== match[3]
            || (match[1] === 'files' && components.length !== 1)) return null;
        const resource = { kind: match[1] as 'files' | 'memory', slug: resourceSlug, path: components.join('/') };
        return slug && resource.slug !== slug ? null : resource;
    } catch { return null; }
}

/** Remove saved grants before inserting media into the live DOM. */
export function prepareResourceHtml(html: string, slug: string): string {
    if (typeof document === 'undefined') return html;
    const template = document.createElement('template');
    template.innerHTML = html;
    for (const node of template.content.querySelectorAll("[data-resource]")) node.removeAttribute("data-resource");
    for (const node of template.content.querySelectorAll<HTMLElement>('[src], a[href]')) {
        const attr = node.hasAttribute('src') ? 'src' : 'href';
        const resource = resourceFromUrl(node.getAttribute(attr) ?? '', slug);
        if (!resource) continue;
        node.removeAttribute(attr);
        node.dataset.resource = JSON.stringify(resource);
    }
    return template.innerHTML;
}

export function resourceProse(node: HTMLElement, _html: string) {
    let stops: (() => void)[] = [];
    let generation = 0;
    let disposed = false;
    function bind(expectedGeneration = generation) {
        if (disposed || expectedGeneration !== generation) return;
        stops.forEach(stop => stop());
        stops = [];
        for (const child of node.querySelectorAll<HTMLElement>('[data-resource]')) {
            const resource: MediaResource = JSON.parse(child.dataset.resource!);
            stops.push(maintainResource(() => issueResource(resource), url => {
                const attr = child.tagName === 'A' ? 'href' : 'src';
                if (url) child.setAttribute(attr, url); else child.removeAttribute(attr);
            }));
        }
    }
    bind();
    return {
        update() {
            const expectedGeneration = ++generation;
            queueMicrotask(() => bind(expectedGeneration));
        },
        destroy() {
            disposed = true;
            generation += 1;
            stops.forEach(stop => stop());
            stops = [];
        },
    };
}
