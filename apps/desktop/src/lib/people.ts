import type { User } from "../bridge-types";

/** Google People API person, the slice `people:searchDirectoryPeople` returns for readMask names,emailAddresses,photos. */
export interface Person {
  resourceName: string; // people/{id}
  names?: { displayName?: string }[];
  emailAddresses?: { value?: string }[];
  photos?: { url?: string }[];
}

/** people/{id} → users/{id}; Chat and People share the id. */
export function personToUser(p: Person): User {
  return {
    name: `users/${p.resourceName.replace(/^people\//, "")}`,
    displayName: p.names?.[0]?.displayName,
    email: p.emailAddresses?.[0]?.value,
    avatarUrl: p.photos?.[0]?.url,
    type: "HUMAN",
  };
}
