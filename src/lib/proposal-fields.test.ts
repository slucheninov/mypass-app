import { describe, expect, it } from "vitest";
import { proposalCustomFields, proposalFieldLabel } from "./proposal-fields";

describe("proposal fields", () => {
  it("names a known kind in the app's words and keeps the page label of an other value", () => {
    expect(proposalFieldLabel({ kind: "postalCode", label: "ZIP" })).toBe("Postal Code");
    expect(proposalFieldLabel({ kind: "birthday", label: "DOB" })).toBe("Birthday");
    expect(proposalFieldLabel({ kind: "other", label: "Nickname" })).toBe("Nickname");
  });

  it("never repeats a key nor takes a standard field name", () => {
    const fields = proposalCustomFields(
      [
        { kind: "phone", label: "Mobile", value: "1" },
        { kind: "phone", label: "Home phone", value: "2" },
        { kind: "other", label: "Password", value: "x" },
        { kind: "other", label: "notes", value: "y" },
        { kind: "other", label: "Pet", value: "z" },
      ],
      [{ key: "Pet", value: "cat", protected: false }],
    );
    expect(fields.map((f) => f.key)).toEqual(["Phone", "Phone 2", "Password 2", "notes 2", "Pet 2"]);
    expect(fields.every((f) => !f.protected)).toBe(true);
  });
});
