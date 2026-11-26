// Where to go after signing in.
export function afterSignIn(next: string | null): string {
  return next ?? "/";
}
