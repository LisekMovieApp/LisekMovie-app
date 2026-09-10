# Internationalization

- Put all user-facing text in `src/lib/i18n/en.json` and `src/lib/i18n/pl.json`, using the same semantic keys and interpolation placeholders in every supported dictionary.
- Render copy through the existing `t()` helper. Do not add language-specific sentences, bilingual helper functions, or `locale === 'pl' ? ... : ...` branches to components or API helpers.
- Store error codes or translation keys in reactive state; translate at render time so messages follow language changes.
- Use interpolation for complete phrases. Keep API identifiers separate from translated option labels.
- Populate language selectors from `SUPPORTED_LOCALES`, with names in the dictionaries. Register additional dictionaries centrally in `translations.ts`.
- Run the translation checks and relevant frontend tests after editing UI copy or adding screens.
