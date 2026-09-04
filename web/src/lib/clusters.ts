export type Cluster = {
  id: string;
  name: string;
  suggested_name: string;
  custom_name: boolean;
  color: string;
  members: string[];
  evidence: { kind: string; label: string; source: string; member_count: number; coverage: number; specificity: number }[];
  predecessors: { id: string; name: string }[];
};

export type ClusterLevel = {
  level: string;
  resolution: number;
  clusters: Cluster[];
  bridges: { person_id: string; primary_cluster: string; secondary_cluster: string; external_share: number }[];
  seed_agreement: number;
  raw_weight_agreement: number;
  raw_cluster_count: number;
  internal_weight_share: number;
  computed_at: string;
};
