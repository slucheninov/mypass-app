// The social sign-in providers MyPass remembers on a login. The names are brands and stay the
// same in every language, so they are not catalog strings.
import type { SignInProvider } from "./bindings";

export const SIGN_IN_PROVIDERS: readonly SignInProvider[] = ["google", "facebook", "apple", "microsoft", "github"];

export const PROVIDER_NAME: Record<SignInProvider, string> = {
  google: "Google",
  facebook: "Facebook",
  apple: "Apple",
  microsoft: "Microsoft",
  github: "GitHub",
};
