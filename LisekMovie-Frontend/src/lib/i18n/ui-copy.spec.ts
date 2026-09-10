import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { parse } from 'svelte/compiler';
import en from './en.json';
import pl from './pl.json';
import { SUPPORTED_LOCALES, translate } from './translations';

const root = fileURLToPath(new URL('../../', import.meta.url));
const files = readdirSync(root, { recursive: true }).filter((f): f is string =>
    typeof f === 'string' && /\.(svelte|ts)$/.test(f) && !/\.(test|spec)\./.test(f));

function strings(node: unknown, path = ''): Record<string, string> {
    if (typeof node === 'string') return { [path]: node };
    if (!node || typeof node !== 'object') return {};
    return Object.assign({}, ...Object.entries(node).map(([k, v]) => strings(v, path ? `${path}.${k}` : k)));
}

describe('UI translation contract', () => {
    it('keeps interpolation placeholders identical in both dictionaries', () => {
        const english = strings(en), polish = strings(pl);
        const placeholders = (s: string) => [...new Set(s.match(/\{\w+\}/g) ?? [])].sort();
        for (const key of Object.keys(english)) {
            expect(placeholders(polish[key]), key).toEqual(placeholders(english[key]));
        }
    });

    it('resolves every literal translation key used by frontend code', () => {
        const missing: string[] = [];
        for (const name of files) {
            const source = readFileSync(join(root, name), 'utf8');
            for (const match of source.matchAll(/\bt\(['"]([\w.]+)['"]\s*[,)]/g)) {
                for (const locale of SUPPORTED_LOCALES) {
                    if (translate(locale, match[1]) === match[1]) missing.push(`${name}: ${locale} ${match[1]}`);
                }
            }
        }
        expect(missing).toEqual([]);
    });

    it('keeps visible Svelte text in dictionaries, except the product name', () => {
        const hardcoded: string[] = [];
        function walk(value: unknown, name: string) {
            if (!value || typeof value !== 'object') return;
            const node = value as Record<string, unknown>;
            if (node.type === 'Attribute' && !['title', 'alt', 'aria-label', 'placeholder'].includes(String(node.name))) return;
            if (node.type === 'Text' && typeof node.data === 'string') {
                const text = node.data.trim();
                if (/\p{L}/u.test(text) && !['LisekMovie', '· LisekMovie'].includes(text)) hardcoded.push(`${name}: ${text}`);
            }
            for (const child of Object.values(node)) {
                if (Array.isArray(child)) child.forEach(v => walk(v, name));
                else if (child && typeof child === 'object') walk(child, name);
            }
        }
        for (const name of files.filter(f => f.endsWith('.svelte'))) {
            walk(parse(readFileSync(join(root, name), 'utf8'), { modern: true }).fragment, name);
        }
        expect(hardcoded).toEqual([]);
    });

    it('does not choose copy with a hardcoded locale branch', () => {
        const offenders = files.filter(name => /getLocale\(\)\s*===?\s*['"](?:pl|en)['"]/.test(readFileSync(join(root, name), 'utf8')));
        expect(offenders).toEqual([]);
    });
});
