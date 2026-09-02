export type OverviewPerson = {
  id: string;
  display_name: string;
  lifecycle_state: "active" | "retired";
  affinity_score: number | null;
  affinity_tier: string | null;
  activity_state: string | null;
  interaction_count: number;
  last_interaction_at: string | null;
  is_self: boolean;
  tags: string[];
  identities: string[];
};

export type GraphNode = { id: string; person_id: string; label: string };
export type GraphEdge = {
  source: string;
  target: string;
  relationship_type: string;
  confidence: number;
};

export type Overview = {
  people: OverviewPerson[];
  graph: { nodes: GraphNode[]; edges: GraphEdge[]; mermaid: string };
};

export type Person = {
  id: string;
  display_name: string;
  apple_contact_id: string | null;
  lifecycle_state: string;
  affinity_score: number | null;
  affinity_tier: string | null;
  activity_state: string | null;
  identities: { kind: string; value: string; is_self: boolean }[];
  notes: { id: string; body: string; created_at: string }[];
  facts: { key: string; value: string }[];
  tags: string[];
};

export type Score = {
  person_id: string;
  display_name: string;
  affinity_score: number;
  affinity_tier: string;
  activity_state: string;
  behavioral_score: number;
  semantic_score: number;
  components: {
    interactions_90d: number;
    active_days_90d: number;
    channels_90d: number;
    incoming_90d: number;
    outgoing_90d: number;
    days_since_last: number | null;
  };
};

export type InteractionPreview = {
  id: string;
  channel: string;
  kind: string;
  occurred_at: string;
  direction: string | null;
  subject: string | null;
  preview: string | null;
  has_body: boolean;
  attachments: { filename: string | null; mime_type: string | null; size_bytes: number | null }[];
};

export type Relationship = {
  id: string;
  person_id: string;
  display_name: string;
  relationship_type: string;
  confidence: number;
  status: string;
  evidence: unknown;
  first_observed_at: string | null;
  last_observed_at: string | null;
};

export type PersonDetail = {
  person: Person;
  score: Score;
  interactions: InteractionPreview[];
  relationships: Relationship[];
  important_dates: { id: string; label: string; date: string; recurring: boolean }[];
  followups: {
    id: string;
    body: string;
    due_at: string | null;
    completed_at: string | null;
    created_at: string;
  }[];
  cadence: { interval_days: number; updated_at: string } | null;
  summaries: { id: string; summary: string; model_version: string; created_at: string }[];
  photo: {
    photos_name: string | null;
    photos_asset_id: string | null;
    state: string;
    reviewed_at: string | null;
    updated_at: string;
  } | null;
};

export type InteractionBody = { id: string; subject: string | null; body: string | null };

export type ActionState = { status: "idle" | "success" | "error"; message: string };
