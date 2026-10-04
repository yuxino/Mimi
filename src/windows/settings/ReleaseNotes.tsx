import type { ReactNode } from "react";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import {
  parseReleaseNotes,
  parseReleaseNotesInline,
  type ReleaseNotesInline,
  type ReleaseNotesLanguage,
} from "./releaseNotesModel";

export function ReleaseNotes({ notes, language = effectiveUiLanguage() }: {
  notes: string;
  language?: ReleaseNotesLanguage;
}) {
  const blocks = parseReleaseNotes(notes, language);
  return (
    <div className="software-update__notes">
      {blocks.length ? blocks.map((block, index) => {
        switch (block.kind) {
          case "heading": return <h4 key={index}>{inline(block.text)}</h4>;
          case "paragraph": return <p key={index}>{inline(block.text)}</p>;
          case "code": return <pre key={index}><code>{block.text}</code></pre>;
          case "list": {
            const List = block.ordered ? "ol" : "ul";
            return <List key={index}>{block.items.map((item, itemIndex) => <li key={itemIndex}>{inline(item)}</li>)}</List>;
          }
        }
      }) : <p>{I18N.settings.noReleaseNotes}</p>}
    </div>
  );
}

function inline(text: string): ReactNode[] {
  return renderInline(parseReleaseNotesInline(text));
}

function renderInline(tokens: ReleaseNotesInline[]): ReactNode[] {
  return tokens.map((token, index) => {
    switch (token.kind) {
      case "text": return token.text;
      case "code": return <code key={index}>{token.text}</code>;
      case "strong": return <strong key={index}>{renderInline(token.children)}</strong>;
      case "emphasis": return <em key={index}>{renderInline(token.children)}</em>;
    }
  });
}
