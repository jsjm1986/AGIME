export const RELATIONSHIP_MEMORY_UPDATED_EVENT =
  "agime:relationship-memory-updated";

export interface RelationshipMemoryPatchPayload {
  preferred_address?: string | null;
  role_hint?: string | null;
  current_focus?: string | null;
  collaboration_preference?: string | null;
  notes?: string | null;
}

export interface RelationshipMemoryUpdatedDetail {
  teamId: string;
  source: "sidebar" | "chat";
  patch?: RelationshipMemoryPatchPayload;
}

export function dispatchRelationshipMemoryUpdated(
  detail: RelationshipMemoryUpdatedDetail,
) {
  if (typeof window === "undefined") {
    return;
  }
  window.dispatchEvent(
    new CustomEvent<RelationshipMemoryUpdatedDetail>(
      RELATIONSHIP_MEMORY_UPDATED_EVENT,
      { detail },
    ),
  );
}
