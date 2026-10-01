use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared::DocumentAccessInfo;
use tracing::warn;

use crate::config::SharePointGraphConfig;

// =============================================================================
// Document Catalog
// =============================================================================
//
// The browser UI, access rules, and prompt-grounding all depend on the same
// normalized document shape. Keeping that normalization here lets us swap the
// backing source from a static demo list to Microsoft Graph without changing
// the rest of the app's contract.

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocumentRecord {
    pub id: String,
    pub title: String,
    pub source: String,
    pub summary: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocumentTreeNode {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub children: Vec<DocumentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCatalog {
    pub tree: Vec<DocumentTreeNode>,
}

impl DocumentCatalog {
    pub fn static_mock() -> Self {
        Self {
            tree: vec![
                DocumentTreeNode {
                    id: "category-account".to_string(),
                    title: "Account".to_string(),
                    kind: "category".to_string(),
                    children: vec![
                        static_document(
                            "sharepoint-q1-plan",
                            "Q1 Delivery Plan",
                            "SharePoint / Account",
                            "Milestones, owners, and target dates for the Q1 rollout.",
                            "Phase 2 depends on finishing the document ingestion worker and validating the approval workflow with operations.",
                        ),
                        static_document(
                            "sharepoint-sharepoint-sync",
                            "SharePoint Sync Overview",
                            "SharePoint / Account",
                            "How synced files are indexed and exposed to the assistant.",
                            "Each document includes a stable source identifier, a short summary, and an excerpt that can be injected into the assistant context for retrieval-free demos.",
                        ),
                        static_document(
                            "sharepoint-customer-renewals",
                            "Customer Renewals Tracker",
                            "SharePoint / Account",
                            "Renewal windows, owners, and risk notes for active customers.",
                            "Accounts marked amber need a commercial review two weeks before renewal and a documented mitigation plan for any unresolved support escalations.",
                        ),
                        static_document(
                            "sharepoint-onboarding-checklist",
                            "Enterprise Onboarding Checklist",
                            "SharePoint / Account",
                            "Checklist used during enterprise account onboarding.",
                            "Confirm security questionnaire completion, environment setup, stakeholder introductions, and the first weekly success review before handoff.",
                        ),
                        static_document(
                            "sharepoint-escalation-matrix",
                            "Escalation Matrix",
                            "SharePoint / Account",
                            "Who to contact for commercial, technical, and delivery escalations.",
                            "Delivery issues older than five business days should be escalated to the program lead and copied to the account owner for customer-facing coordination.",
                        ),
                        static_document(
                            "sharepoint-sla-overview",
                            "SLA Overview",
                            "SharePoint / Account",
                            "Response-time commitments and customer communication rules.",
                            "Priority-one incidents require an acknowledgment within fifteen minutes and an external update cadence of no more than sixty minutes.",
                        ),
                    ],
                },
                DocumentTreeNode {
                    id: "category-law".to_string(),
                    title: "Law".to_string(),
                    kind: "category".to_string(),
                    children: vec![
                        static_document(
                            "sharepoint-rust-guidelines",
                            "Rust Service Guidelines",
                            "SharePoint / Law",
                            "Standards for async handlers, tracing, and error design in Rust services.",
                            "Prefer request-scoped tracing spans, typed domain errors, and explicit fallbacks when external APIs are mocked in development.",
                        ),
                        static_document(
                            "sharepoint-data-retention-policy",
                            "Data Retention Policy",
                            "SharePoint / Law",
                            "Retention windows for customer data and audit material.",
                            "Conversation data used for support or legal review must be retained according to the active customer schedule and deleted when the retention period expires.",
                        ),
                        static_document(
                            "sharepoint-dpa-template",
                            "DPA Template Notes",
                            "SharePoint / Law",
                            "Negotiation guidance for data processing agreements.",
                            "Any request to localize storage by geography must be reviewed with engineering before commercial approval is shared with the customer.",
                        ),
                        static_document(
                            "sharepoint-privacy-review",
                            "Privacy Review Checklist",
                            "SharePoint / Law",
                            "Checklist used before launching features that handle customer documents.",
                            "Document-grounded features should specify data source, persistence duration, user visibility, and admin override behavior before release.",
                        ),
                        static_document(
                            "sharepoint-contract-fallbacks",
                            "Contract Fallback Clauses",
                            "SharePoint / Law",
                            "Approved fallback language for procurement redlines.",
                            "Where unlimited indemnity is requested, legal should propose the capped fallback language before the deal desk responds to the customer.",
                        ),
                    ],
                },
                DocumentTreeNode {
                    id: "category-hr".to_string(),
                    title: "HR".to_string(),
                    kind: "category".to_string(),
                    children: vec![
                        static_document(
                            "sharepoint-leptos-ux",
                            "Leptos UX Notes",
                            "SharePoint / HR",
                            "Interaction notes for chat layouts, mentions, and streaming affordances.",
                            "When the user types @, surface relevant document suggestions inline so context selection feels part of composition rather than a separate workflow.",
                        ),
                        static_document(
                            "sharepoint-wasm-brief",
                            "WASM Performance Brief",
                            "SharePoint / HR",
                            "Benchmarks and guidance for keeping WebAssembly payloads responsive.",
                            "Prioritize reducing hydration dependencies and streaming meaningful HTML first when page interactivity can tolerate plain JavaScript during early iterations.",
                        ),
                        static_document(
                            "sharepoint-remote-work-guide",
                            "Remote Work Guide",
                            "SharePoint / HR",
                            "Expectations for remote collaboration and communication cadence.",
                            "Distributed teams should default to written updates, explicit ownership, and visible working notes so context survives across time zones.",
                        ),
                        static_document(
                            "sharepoint-manager-handbook",
                            "Manager Handbook",
                            "SharePoint / HR",
                            "Coaching, feedback, and planning guidance for people managers.",
                            "Managers are expected to document role expectations, growth areas, and concrete follow-ups after each monthly one-on-one.",
                        ),
                        static_document(
                            "sharepoint-interview-rubric",
                            "Interview Rubric",
                            "SharePoint / HR",
                            "Evaluation dimensions and score guidance for hiring loops.",
                            "Interview feedback should separate observed evidence from interpretation and avoid using confidence or polish as a proxy for technical strength.",
                        ),
                        static_document(
                            "sharepoint-benefits-faq",
                            "Benefits FAQ",
                            "SharePoint / HR",
                            "Answers to recurring employee questions about benefits.",
                            "Eligibility windows, enrollment deadlines, and escalation contacts should be shared exactly as written to avoid plan-specific confusion.",
                        ),
                        static_document(
                            "sharepoint-leave-policy",
                            "Leave Policy",
                            "SharePoint / HR",
                            "Vacation, parental leave, and medical leave guidance.",
                            "Extended leave requests should be routed through the documented HR workflow and never handled only through informal manager approval.",
                        ),
                    ],
                },
            ],
        }
    }

    pub fn all_documents(&self) -> Vec<DocumentRecord> {
        self.tree
            .iter()
            .flat_map(|node| node.children.iter().cloned())
            .collect()
    }

    pub fn find_documents(
        &self,
        ids: &[String],
        allowed_document_ids: &HashSet<String>,
    ) -> Vec<DocumentRecord> {
        let documents = self.all_documents();
        ids.iter()
            .filter(|id| allowed_document_ids.contains(id.as_str()))
            .filter_map(|id| documents.iter().find(|doc| doc.id == *id).cloned())
            .collect()
    }

    pub fn filter_document_tree(
        &self,
        allowed_document_ids: &HashSet<String>,
    ) -> Vec<DocumentTreeNode> {
        self.tree
            .iter()
            .filter_map(|node| {
                let children = node
                    .children
                    .iter()
                    .filter(|document| allowed_document_ids.contains(document.id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();

                if children.is_empty() {
                    None
                } else {
                    Some(DocumentTreeNode {
                        id: node.id.clone(),
                        title: node.title.clone(),
                        kind: node.kind.clone(),
                        children,
                    })
                }
            })
            .collect()
    }
}

fn static_document(
    id: &str,
    title: &str,
    source: &str,
    summary: &str,
    snippet: &str,
) -> DocumentRecord {
    DocumentRecord {
        id: id.to_string(),
        title: title.to_string(),
        source: source.to_string(),
        summary: summary.to_string(),
        snippet: snippet.to_string(),
    }
}

// =============================================================================
// SharePoint Graph Loader
// =============================================================================
//
// We normalize top-level folders into UI categories and files into documents.
// A file's summary prefers SharePoint metadata when present, then falls back to
// the first extracted text line. Binary formats keep a metadata-only snippet
// until a richer extractor is added.

#[derive(Clone)]
pub struct DocumentService {
    client: Client,
    graph: Option<SharePointGraphConfig>,
}

impl DocumentService {
    pub fn new(graph: Option<SharePointGraphConfig>) -> Self {
        Self {
            client: Client::new(),
            graph,
        }
    }

    pub async fn load_catalog(&self) -> Result<DocumentCatalog> {
        match &self.graph {
            Some(config) => self
                .load_graph_catalog(config)
                .await
                .context("while attempting to load documents from SharePoint Graph"),
            None => {
                warn!("SharePoint Graph is not configured; using the static document catalog");
                Ok(DocumentCatalog::static_mock())
            }
        }
    }

    async fn load_graph_catalog(&self, config: &SharePointGraphConfig) -> Result<DocumentCatalog> {
        let token = self.fetch_access_token(config).await?;
        let items = self
            .walk_drive_items(config, &token, config.root_item_id.as_deref(), &[])
            .await?;

        let mut categories = HashMap::<String, DocumentTreeNode>::new();
        for item in items {
            if item.folder.is_some() {
                continue;
            }

            let Some(top_level_folder) = item
                .path_segments
                .iter()
                .find(|segment| !segment.trim().is_empty())
                .cloned()
            else {
                continue;
            };

            let category_id = format!("category-{}", slugify(&top_level_folder));
            let category_title = top_level_folder.clone();
            let source_suffix = item
                .path_segments
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<_>>();
            let source = if source_suffix.is_empty() {
                format!("SharePoint / {category_title}")
            } else {
                format!(
                    "SharePoint / {} / {}",
                    category_title,
                    source_suffix.join(" / ")
                )
            };

            let extracted_text = self
                .fetch_document_text(config, &token, &item)
                .await
                .with_context(|| format!("while attempting to extract {}", item.name))?;
            let excerpt = excerpt_text(extracted_text.as_deref(), 280).unwrap_or_else(|| {
                item.web_url
                    .as_deref()
                    .map(|url| format!("Open the original SharePoint document: {url}"))
                    .unwrap_or_else(|| {
                        "Open the original SharePoint document to inspect its full contents."
                            .to_string()
                    })
            });
            let summary = item
                .description
                .clone()
                .or_else(|| summary_from_text(extracted_text.as_deref()))
                .unwrap_or_else(|| {
                    "SharePoint document synced through Microsoft Graph.".to_string()
                });

            categories
                .entry(category_id.clone())
                .or_insert_with(|| DocumentTreeNode {
                    id: category_id.clone(),
                    title: category_title.clone(),
                    kind: "category".to_string(),
                    children: Vec::new(),
                })
                .children
                .push(DocumentRecord {
                    id: format!(
                        "sharepoint-{}",
                        slugify(
                            &item
                                .full_display_path()
                                .unwrap_or_else(|| item.name.clone())
                        )
                    ),
                    title: item.name.clone(),
                    source,
                    summary,
                    snippet: excerpt,
                });
        }

        let mut tree = categories.into_values().collect::<Vec<_>>();
        tree.sort_by(|left, right| left.title.cmp(&right.title));
        for node in &mut tree {
            node.children
                .sort_by(|left, right| left.title.cmp(&right.title));
        }

        Ok(DocumentCatalog { tree })
    }

    async fn fetch_access_token(&self, config: &SharePointGraphConfig) -> Result<String> {
        let params = [
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("grant_type", "client_credentials"),
            ("scope", "https://graph.microsoft.com/.default"),
        ];
        let url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            config.tenant_id
        );

        let response = self
            .client
            .post(url)
            .form(&params)
            .send()
            .await
            .context("while attempting to request a Microsoft Graph access token")?;
        let response = response
            .error_for_status()
            .context("while attempting to validate the Microsoft Graph token response")?;
        let payload = response
            .json::<GraphTokenResponse>()
            .await
            .context("while attempting to deserialize the Microsoft Graph token response")?;

        Ok(payload.access_token)
    }

    async fn walk_drive_items(
        &self,
        config: &SharePointGraphConfig,
        token: &str,
        parent_item_id: Option<&str>,
        path_segments: &[String],
    ) -> Result<Vec<GraphDriveItem>> {
        let endpoint = match parent_item_id {
            Some(item_id) => format!(
                "https://graph.microsoft.com/v1.0/sites/{}/drives/{}/items/{item_id}/children",
                config.site_id, config.drive_id
            ),
            None => format!(
                "https://graph.microsoft.com/v1.0/sites/{}/drives/{}/root/children",
                config.site_id, config.drive_id
            ),
        };
        let mut page_url = reqwest::Url::parse(&endpoint)
            .context("while attempting to parse the Microsoft Graph drive listing URL")?;
        page_url.query_pairs_mut().append_pair(
            "$select",
            "id,name,description,webUrl,folder,file,parentReference",
        );

        let mut collected = Vec::new();
        loop {
            let page = self.fetch_graph_page(token, page_url.clone()).await?;
            for mut item in page.value {
                let mut child_path = path_segments.to_vec();
                if item.folder.is_some() {
                    child_path.push(item.name.clone());
                }

                item.path_segments = path_segments.to_vec();
                collected.push(item.clone());

                if item.folder.is_some() {
                    let nested =
                        Box::pin(self.walk_drive_items(config, token, Some(&item.id), &child_path))
                            .await?;
                    collected.extend(nested);
                }
            }

            let Some(next_link) = page.next_link else {
                break;
            };
            page_url = reqwest::Url::parse(&next_link)
                .context("while attempting to parse the next Microsoft Graph page URL")?;
        }

        Ok(collected)
    }

    async fn fetch_graph_page(&self, token: &str, url: reqwest::Url) -> Result<GraphListResponse> {
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("while attempting to request a Microsoft Graph drive page")?;
        let response = response
            .error_for_status()
            .context("while attempting to validate the Microsoft Graph drive page response")?;

        response
            .json::<GraphListResponse>()
            .await
            .context("while attempting to deserialize a Microsoft Graph drive page")
    }

    async fn fetch_document_text(
        &self,
        config: &SharePointGraphConfig,
        token: &str,
        item: &GraphDriveItem,
    ) -> Result<Option<String>> {
        let Some(file) = &item.file else {
            return Ok(None);
        };
        let Some(mime_type) = &file.mime_type else {
            return Ok(None);
        };
        if !supports_text_extraction(mime_type, &item.name) {
            return Ok(None);
        }

        let url = format!(
            "https://graph.microsoft.com/v1.0/sites/{}/drives/{}/items/{}/content",
            config.site_id, config.drive_id, item.id
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .with_context(|| format!("while attempting to download {}", item.name))?;
        let response = response.error_for_status().with_context(|| {
            format!(
                "while attempting to validate the download for {}",
                item.name
            )
        })?;
        let body = response.text().await.with_context(|| {
            format!(
                "while attempting to read the downloaded body for {}",
                item.name
            )
        })?;

        Ok(Some(body))
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct DocumentGrant {
    #[serde(default)]
    pub documents: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct DocumentAccessRules {
    #[serde(default)]
    pub users: HashMap<String, DocumentGrant>,
    #[serde(default)]
    pub default: DocumentGrant,
}

pub fn inject_documents_into_prompt(prompt: &str, documents: &[DocumentRecord]) -> String {
    if documents.is_empty() {
        return prompt.to_string();
    }

    let mut context = String::from("Use the selected SharePoint documents as context.\n\n");
    for document in documents {
        context.push_str(&format!(
            "[Document: {} | {}]\nSummary: {}\nExcerpt: {}\n\n",
            document.title, document.source, document.summary, document.snippet
        ));
    }
    context.push_str("User question:\n");
    context.push_str(prompt);
    context
}

impl DocumentAccessRules {
    pub fn from_json(raw: &str) -> Result<Self> {
        serde_json::from_str(raw).context("while attempting to parse DOCUMENT_ACCESS_RULES")
    }

    pub fn from_yaml(raw: &str) -> Result<Self> {
        serde_yaml::from_str(raw).context("while attempting to parse DOCUMENT_ACCESS_RULES_FILE")
    }

    pub fn resolve_for_email(&self, catalog: &DocumentCatalog, email: &str) -> DocumentAccessInfo {
        let grant = self.users.get(email).unwrap_or(&self.default);
        let category_ids = grant
            .categories
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        let explicit_document_ids = grant
            .documents
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();

        let mut document_ids = explicit_document_ids;
        for node in &catalog.tree {
            if category_ids.contains(node.id.as_str()) {
                for document in &node.children {
                    document_ids.insert(document.id.clone());
                }
            }
        }

        DocumentAccessInfo {
            document_ids: document_ids.into_iter().collect(),
            category_ids: category_ids.into_iter().collect(),
        }
    }
}

pub fn allowed_document_id_set(access: &DocumentAccessInfo) -> HashSet<String> {
    access.document_ids.iter().cloned().collect()
}

#[derive(Debug, Clone, Deserialize)]
struct GraphTokenResponse {
    access_token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphListResponse {
    value: Vec<GraphDriveItem>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphDriveItem {
    id: String,
    name: String,
    description: Option<String>,
    #[serde(rename = "webUrl")]
    web_url: Option<String>,
    folder: Option<GraphFolderFacet>,
    file: Option<GraphFileFacet>,
    #[serde(skip)]
    path_segments: Vec<String>,
}

impl GraphDriveItem {
    fn full_display_path(&self) -> Option<String> {
        if self.path_segments.is_empty() {
            None
        } else {
            Some(format!("{}/{}", self.path_segments.join("/"), self.name))
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GraphFolderFacet {}

#[derive(Debug, Clone, Deserialize)]
struct GraphFileFacet {
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

fn supports_text_extraction(mime_type: &str, name: &str) -> bool {
    let lower_name = name.to_ascii_lowercase();
    mime_type.starts_with("text/")
        || mime_type == "application/json"
        || mime_type == "application/xml"
        || mime_type == "application/javascript"
        || lower_name.ends_with(".md")
        || lower_name.ends_with(".txt")
        || lower_name.ends_with(".json")
        || lower_name.ends_with(".csv")
        || lower_name.ends_with(".html")
        || lower_name.ends_with(".xml")
}

fn summary_from_text(text: Option<&str>) -> Option<String> {
    text.and_then(|value| {
        value
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| truncate_with_ellipsis(line, 120))
    })
}

fn excerpt_text(text: Option<&str>, limit: usize) -> Option<String> {
    text.map(|value| {
        let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
        truncate_with_ellipsis(&collapsed, limit)
    })
    .filter(|value| !value.is_empty())
}

fn truncate_with_ellipsis(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    let mut truncated = trimmed.chars().take(limit).collect::<String>();
    if trimmed.chars().count() > limit {
        truncated.push_str("...");
    }
    truncated
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in value.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentAccessRules, DocumentCatalog, GraphDriveItem, GraphFileFacet,
        allowed_document_id_set, slugify, supports_text_extraction,
    };

    #[test]
    fn rules_expand_categories_and_documents() {
        let rules = DocumentAccessRules::from_json(
            r#"{
                "users": {
                    "alice@company.com": {
                        "documents": ["sharepoint-leave-policy"],
                        "categories": ["category-account"]
                    }
                },
                "default": {
                    "documents": [],
                    "categories": []
                }
            }"#,
        )
        .expect("rules should parse");

        let access = rules.resolve_for_email(&DocumentCatalog::static_mock(), "alice@company.com");

        assert!(
            access
                .category_ids
                .iter()
                .any(|id| id == "category-account")
        );
        assert!(
            access
                .document_ids
                .iter()
                .any(|id| id == "sharepoint-q1-plan")
        );
        assert!(
            access
                .document_ids
                .iter()
                .any(|id| id == "sharepoint-leave-policy")
        );
    }

    #[test]
    fn unknown_user_uses_default_grants() {
        let rules = DocumentAccessRules::from_json(
            r#"{
                "default": {
                    "documents": ["sharepoint-privacy-review"],
                    "categories": []
                }
            }"#,
        )
        .expect("rules should parse");

        let access = rules.resolve_for_email(&DocumentCatalog::static_mock(), "nobody@company.com");

        assert_eq!(
            access.document_ids,
            vec!["sharepoint-privacy-review".to_string()]
        );
    }

    #[test]
    fn document_tree_filters_unauthorized_documents() {
        let rules = DocumentAccessRules::from_json(
            r#"{
                "default": {
                    "documents": ["sharepoint-privacy-review"],
                    "categories": []
                }
            }"#,
        )
        .expect("rules should parse");
        let catalog = DocumentCatalog::static_mock();
        let access = rules.resolve_for_email(&catalog, "anyone@company.com");
        let filtered = catalog.filter_document_tree(&allowed_document_id_set(&access));

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].children.len(), 1);
        assert_eq!(filtered[0].children[0].id, "sharepoint-privacy-review");
    }

    #[test]
    fn slugify_builds_stable_sharepoint_ids() {
        assert_eq!(
            slugify("Account / Q1 Delivery Plan.md"),
            "account-q1-delivery-plan-md"
        );
    }

    #[test]
    fn text_extraction_only_runs_for_plaintext_like_files() {
        assert!(supports_text_extraction("text/plain", "notes.txt"));
        assert!(supports_text_extraction("application/json", "notes.json"));
        assert!(!supports_text_extraction("application/pdf", "contract.pdf"));
    }

    #[test]
    fn graph_item_display_path_uses_parent_segments() {
        let item = GraphDriveItem {
            id: "1".to_string(),
            name: "Policy.md".to_string(),
            description: None,
            web_url: None,
            folder: None,
            file: Some(GraphFileFacet {
                mime_type: Some("text/markdown".to_string()),
            }),
            path_segments: vec!["HR".to_string(), "Policies".to_string()],
        };

        assert_eq!(
            item.full_display_path().as_deref(),
            Some("HR/Policies/Policy.md")
        );
    }
}
