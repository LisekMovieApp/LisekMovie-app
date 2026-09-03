export { setLocale, getLocale, t } from './locale.svelte';
export {
	SUPPORTED_LOCALES,
	DEFAULT_LOCALE,
	isSupportedLocale,
	translate,
	type Locale
} from './translations';
export { LOCALE_COOKIE_NAME, setLocaleCookie, parseLocaleCookie } from './cookie';
