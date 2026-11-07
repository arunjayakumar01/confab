export function uuid(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === "x" ? r : (r & 0x3) | 0x8).toString(16);
  });
}
/** "spaces/AAA/messages/x" → "spaces/AAA" */
export function spaceOf(messageName: string): string {
  return messageName.split("/").slice(0, 2).join("/");
}
export function lastSegment(name: string): string {
  return name.slice(name.lastIndexOf("/") + 1);
}
export function isClientName(name: string): boolean {
  return lastSegment(name).startsWith("client-");
}
export function clientIdOf(name: string): string | null {
  const seg = lastSegment(name);
  return seg.startsWith("client-") ? seg.slice("client-".length) : null;
}
