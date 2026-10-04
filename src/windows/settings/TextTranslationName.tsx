import { I18N } from "../../lib/i18n";
import { defaultTextTranslationName } from "../../lib/textTranslationName";
import type { ServiceProfile, TextTranslationNameDraft } from "../../lib/types";
import { AutoSaveNameField } from "./AutoSaveNameField";

/** Display metadata is saved independently of endpoint, model and credential drafts. */
export function TextTranslationName({ profile, route, inputId, disabled, onSave }: {
  profile: ServiceProfile;
  route: TextTranslationNameDraft["route"];
  inputId: string;
  disabled: boolean;
  onSave: (route: TextTranslationNameDraft["route"], name: string) => Promise<unknown>;
}) {
  return <AutoSaveNameField key={`${profile.id}:${route}`} id={inputId} label={I18N.settings.textTranslationName}
    className="translation-name-field" value={profile.textTranslationNames?.[route] ?? ""}
    placeholder={defaultTextTranslationName(route)} disabled={disabled} allowEmpty
    onSave={name => onSave(route, name)} />;
}
