use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaClassification {
    /// Sufficient documented rights; full commercial permission verified
    Cleared,
    /// Verified original production asset (Unreal Engine studio, MetaHuman anchors, original shaders)
    Original,
    /// Documented legal-exception/fair-use analysis reviewed and accepted
    ExceptionReviewed,
    /// Rights or exception uncertain; held pending human Telegram clearance
    ReviewRequired,
    /// Conditions or permissions not satisfied; must be replaced or omitted
    Restricted,
    /// Known unauthorized or prohibited use; blocked permanently
    Prohibited,
}

impl MediaClassification {
    pub fn is_allowed_in_render(&self) -> bool {
        matches!(
            self,
            Self::Cleared | Self::Original | Self::ExceptionReviewed
        )
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Cleared => "Sufficient documented rights (Allowed)",
            Self::Original => "Verified original production asset (Allowed)",
            Self::ExceptionReviewed => "Documented legal-exception accepted (Allowed with risk record)",
            Self::ReviewRequired => "Rights or exception uncertain (Held for human review)",
            Self::Restricted => "Conditions not satisfied (Replace or block)",
            Self::Prohibited => "Unauthorized or disallowed use (Permanently blocked)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    OriginalUnrealFootage,
    OriginalMetaHuman,
    StudioPromotional,
    Trailer,
    EpisodeOrMovieClip,
    ScreenshotOrKeyArt,
    MusicOrSfx,
    CreativeCommons,
    PublicDomain,
    AiGenerated,
    FanArt,
    LogoOrTrademark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FairUseAssessment {
    /// Factor 1: Purpose & character of use (transformative criticism, news reporting)
    pub purpose_and_character: String,
    /// Detailed transformation explanation (how anchor dialogue provides new context)
    pub transformative_nature: String,
    /// Factor 2: Nature of copyrighted work (published promotional, broadcast)
    pub nature_of_copyrighted_work: String,
    /// Factor 3: Amount and substantiality (only necessary excerpt, trailer audio muted)
    pub amount_and_substantiality: String,
    /// Factor 4: Market effect (no substitute for viewing the official release)
    pub effect_on_market: String,
    /// Exact editorial connection to spoken anchor dialogue
    pub editorial_justification: String,
}

impl FairUseAssessment {
    pub fn is_defensible(&self) -> bool {
        !self.purpose_and_character.trim().is_empty()
            && !self.transformative_nature.trim().is_empty()
            && !self.amount_and_substantiality.trim().is_empty()
            && !self.effect_on_market.trim().is_empty()
            && !self.editorial_justification.trim().is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RightsRecord {
    pub asset_id: String,
    pub sha256: String,
    pub media_type: MediaType,
    pub creator_or_holder: String,
    pub source_url: Option<String>,
    pub acquisition_timestamp: DateTime<Utc>,
    pub license_type: String,
    pub commercial_use_permitted: bool,
    pub restrictions: Vec<String>,
    pub classification: MediaClassification,
    pub fair_use_assessment: Option<FairUseAssessment>,
    pub audio_muted_for_clearance: bool,
}

impl RightsRecord {
    pub fn mock_original(asset_id: &str) -> Self {
        Self {
            asset_id: asset_id.to_string(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            media_type: MediaType::OriginalUnrealFootage,
            creator_or_holder: "Clearly Drawn News Virtual Studio".into(),
            source_url: None,
            acquisition_timestamp: Utc::now(),
            license_type: "Proprietary Original".into(),
            commercial_use_permitted: true,
            restrictions: vec![],
            classification: MediaClassification::Original,
            fair_use_assessment: None,
            audio_muted_for_clearance: false,
        }
    }

    pub fn mock_prohibited(asset_id: &str) -> Self {
        Self {
            asset_id: asset_id.to_string(),
            sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
            media_type: MediaType::EpisodeOrMovieClip,
            creator_or_holder: "Unknown Third Party".into(),
            source_url: Some("https://unauthorized-stream.com".into()),
            acquisition_timestamp: Utc::now(),
            license_type: "None".into(),
            commercial_use_permitted: false,
            restrictions: vec!["Prohibited full broadcast rip".into()],
            classification: MediaClassification::Prohibited,
            fair_use_assessment: None,
            audio_muted_for_clearance: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanResult {
    AllClear,
    ReviewPending(Vec<String>),
    BlockedRestrictedAssets(Vec<String>),
    BlockedProhibitedAssets(Vec<String>),
}

#[derive(Default)]
pub struct CopyrightComplianceEngine;

impl CopyrightComplianceEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn is_cleared_for_render(&self, record: &RightsRecord) -> bool {
        record.classification.is_allowed_in_render()
    }

    pub fn scan_production_manifest(&self, manifest: &[RightsRecord]) -> ScanResult {
        let mut prohibited = Vec::new();
        let mut restricted = Vec::new();
        let mut review_needed = Vec::new();

        for record in manifest {
            match record.classification {
                MediaClassification::Prohibited => prohibited.push(record.asset_id.clone()),
                MediaClassification::Restricted => restricted.push(record.asset_id.clone()),
                MediaClassification::ReviewRequired => review_needed.push(record.asset_id.clone()),
                MediaClassification::Cleared
                | MediaClassification::Original
                | MediaClassification::ExceptionReviewed => {}
            }
        }

        if !prohibited.is_empty() {
            ScanResult::BlockedProhibitedAssets(prohibited)
        } else if !restricted.is_empty() {
            ScanResult::BlockedRestrictedAssets(restricted)
        } else if !review_needed.is_empty() {
            ScanResult::ReviewPending(review_needed)
        } else {
            ScanResult::AllClear
        }
    }

    pub fn create_fair_use_trailer_record(
        &self,
        asset_id: &str,
        sha256: &str,
        source_url: &str,
        studio_or_rights_holder: &str,
        editorial_justification: &str,
    ) -> RightsRecord {
        let assessment = FairUseAssessment {
            purpose_and_character: "Non-substitutive animation news reporting, visual critique, and artistic commentary.".into(),
            transformative_nature: "Excerpt is presented within the Unreal Engine virtual news studio on digital video wall with spoken critical anchor analysis.".into(),
            nature_of_copyrighted_work: "Published promotional animation teaser/trailer distributed publicly for promotional marketing.".into(),
            amount_and_substantiality: "Short excerpt strictly limited to demonstrated artistic points; trailer soundtrack is completely muted to eliminate music rights claims.".into(),
            effect_on_market: "Promotional review encourages official viewership and does not compete with or replace watching the completed series/film.".into(),
            editorial_justification: editorial_justification.to_string(),
        };

        RightsRecord {
            asset_id: asset_id.to_string(),
            sha256: sha256.to_string(),
            media_type: MediaType::Trailer,
            creator_or_holder: studio_or_rights_holder.to_string(),
            source_url: Some(source_url.to_string()),
            acquisition_timestamp: Utc::now(),
            license_type: "Fair Use 17 U.S.C. §107 (News Reporting / Commentary)".into(),
            commercial_use_permitted: true,
            restrictions: vec![
                "Original audio soundtrack must remain muted".into(),
                "Must be accompanied by anchor voiceover commentary".into(),
            ],
            classification: MediaClassification::ExceptionReviewed,
            fair_use_assessment: Some(assessment),
            audio_muted_for_clearance: true,
        }
    }
}
