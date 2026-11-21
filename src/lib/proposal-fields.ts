// Names the extra values of a registration form (a browser save proposal) as custom fields of the
// new login. A known kind gets the app's own word for it, an `other` value keeps the label the page
// gave it (checked by the bridge: one clean line, no reserved prefix). Keys never repeat and never
// take a standard KeePass or MyPass field name, so the entry always saves.
import type { CustomFieldEdit, ProposalField, ProposalFieldKind } from "./bindings";
import { t } from "./i18n/index.svelte";

/** The field names an entry keeps for itself (`STANDARD_KEYS` in `mypass-session`), compared case-insensitively. */
const RESERVED = ["title", "username", "password", "url", "notes", "otp", "favorite"];

/** The readable name of one extra value. */
export function proposalFieldLabel(field: Pick<ProposalField, "kind" | "label">): string {
  const kind: ProposalFieldKind = field.kind;
  switch (kind) {
    case "name":
      return t("Full Name");
    case "givenName":
      return t("Given Name");
    case "familyName":
      return t("Family Name");
    case "email":
      return t("Email");
    case "phone":
      return t("Phone");
    case "birthday":
      return t("Birthday");
    case "username":
      return t("Username");
    case "organization":
      return t("Organization");
    case "addressLine1":
      return t("Address Line 1");
    case "addressLine2":
      return t("Address Line 2");
    case "city":
      return t("City");
    case "region":
      return t("Region");
    case "postalCode":
      return t("Postal Code");
    case "country":
      return t("Country");
    case "other":
      return field.label;
  }
}

/**
 * Whether the review ticks a value at first: the identity and address values the extension
 * recognised are kept by default, a field the page labelled itself (`other`) is opt-in, because
 * its label is the page's word and its content may be anything.
 */
export function keptByDefault(field: Pick<ProposalField, "kind">): boolean {
  return field.kind !== "other";
}

/** The custom fields the kept extra values become: unique keys, plain (not protected) values. */
export function proposalCustomFields(fields: readonly ProposalField[], existing: readonly CustomFieldEdit[] = []): CustomFieldEdit[] {
  const taken = new Set([...RESERVED, ...existing.map((f) => f.key.toLowerCase())]);
  return fields.map((field) => {
    const base = proposalFieldLabel(field).trim();
    let key = base;
    for (let n = 2; taken.has(key.toLowerCase()); n++) key = `${base} ${n}`;
    taken.add(key.toLowerCase());
    return { key, value: field.value, protected: false };
  });
}
