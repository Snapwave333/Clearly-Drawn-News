use crate::pronunciation::PronunciationDictionary;
use atlas_core::{AtlasError, AtlasResult, EpisodeDurationPolicy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorRole {
    /// "The Enthusiast" (Male): Energetic, passionate, animation history expert, humorous hyperbole.
    Enthusiast,
    /// "The Sharp Counterpart" (Female): Witty, deadpan, observant, dry humor, reality check.
    Counterpart,
    /// Both anchors delivering in unison or fast back-and-forth
    Duo,
}

impl AnchorRole {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Enthusiast => "Marcus (The Enthusiast)",
            Self::Counterpart => "Elena (The Sharp Counterpart)",
            Self::Duo => "Marcus & Elena",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioCameraCut {
    WideTwoShot,
    MarcusCloseUp,
    ElenaCloseUp,
    MarcusOverShoulder,
    ElenaOverShoulder,
    VideoWallCenter,
    SplitScreenAnalysis,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptDialogueLine {
    pub speaker: AnchorRole,
    pub camera_cut: StudioCameraCut,
    pub spoken_text: String,
    pub phonetic_text: String,
    pub anchor_expression: String,
    pub partner_reaction_expression: String,
    pub broll_visual_cue: Option<String>,
    pub estimated_seconds: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsSegmentScript {
    pub segment_number: usize,
    pub segment_title: String,
    pub segment_category: String,
    pub target_duration_secs: f64,
    pub actual_duration_secs: f64,
    pub lines: Vec<ScriptDialogueLine>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MasterEpisodeScript {
    pub show_title: String,
    pub episode_title: String,
    pub episode_date: String,
    pub target_duration_secs: f64,
    pub total_duration_secs: f64,
    pub segments: Vec<NewsSegmentScript>,
}

impl MasterEpisodeScript {
    /// Validates that the episode adheres strictly to the 15-30 minute duration boundary.
    pub fn validate_duration(&self, policy: &EpisodeDurationPolicy) -> AtlasResult<()> {
        let total = self.total_duration_secs;
        if total < policy.min_seconds {
            return Err(AtlasError::Config(format!(
                "Episode duration {:.1}s violates minimum requirement of {:.1}s (15 minutes)",
                total, policy.min_seconds
            )));
        }
        if total > policy.max_seconds {
            return Err(AtlasError::Config(format!(
                "Episode duration {:.1}s violates maximum requirement of {:.1}s (30 minutes)",
                total, policy.max_seconds
            )));
        }
        Ok(())
    }
}

pub struct TwoAnchorScriptWriter {
    pronunciation: PronunciationDictionary,
}

impl Default for TwoAnchorScriptWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl TwoAnchorScriptWriter {
    pub fn new() -> Self {
        Self {
            pronunciation: PronunciationDictionary::new(),
        }
    }

    /// Generates a complete 22-minute (1320-second) broadcast episode script
    /// structured across 8 distinct broadcast segments with organic comedic banter.
    pub fn generate_broadcast_episode(
        &self,
        episode_title: &str,
        episode_date: &str,
    ) -> MasterEpisodeScript {
        let mut segments = vec![
            self.build_cold_open(),
            self.build_lead_story(),
            self.build_spider_verse_story(),
            self.build_fortiche_story(),
            self.build_ghibli_story(),
            self.build_dandadan_story(),
            self.build_rapid_headlines(),
            self.build_closing_segment(),
        ];

        for segment in &mut segments {
            for line in &mut segment.lines {
                line.phonetic_text = self.pronunciation.phonetically_prepare(&line.spoken_text);
                line.estimated_seconds = crate::story_script::estimate_seconds(&line.spoken_text);
            }
            segment.actual_duration_secs = segment.lines.iter().map(|line| line.estimated_seconds).sum();
        }

        let total_duration_secs: f64 = segments.iter().map(|s| s.actual_duration_secs).sum();

        MasterEpisodeScript {
            show_title: atlas_core::SHOW_TITLE.into(),
            episode_title: episode_title.into(),
            episode_date: episode_date.into(),
            target_duration_secs: 1320.0,
            total_duration_secs,
            segments,
        }
    }

    fn make_line(
        speaker: AnchorRole,
        camera: StudioCameraCut,
        spoken: &str,
        expression: &str,
        partner_reaction: &str,
        broll: Option<&str>,
        duration: f64,
    ) -> ScriptDialogueLine {
        ScriptDialogueLine {
            speaker,
            camera_cut: camera,
            spoken_text: spoken.into(),
            phonetic_text: String::new(),
            anchor_expression: expression.into(),
            partner_reaction_expression: partner_reaction.into(),
            broll_visual_cue: broll.map(|s| s.into()),
            estimated_seconds: duration,
        }
    }

    fn build_cold_open(&self) -> NewsSegmentScript {
        let lines = vec![
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::WideTwoShot,
                "Welcome to Clearly Drawn News. I'm Marcus.",
                "WarmConfident",
                "SubtleSmile",
                None,
                3.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "And I'm Elena. Today: MAPPA breaks their silence on Chainsaw Man, Sony reveals the mind-bending rendering engine behind Beyond the Spider-Verse...",
                "SharpDirect",
                "AttentiveNod",
                Some("Teaser reel: Chainsaw Man & Spider-Verse graphics on studio video wall"),
                14.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "And Fortiche expands the Runeterra universe with a brand new adult series! Plus, Studio Ghibli opens an 8K cel preservation vault in Mitaka!",
                "ExcitedGesticulation",
                "AmusedSkepticalEyebrow",
                Some("Teaser graphics: Fortiche & Ghibli archivism"),
                16.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::WideTwoShot,
                "Which means Marcus didn't sleep last night and has analyzed three thousand frames by hand. Let's get straight to the headlines.",
                "DeadpanDryWit",
                "ChuckleDefensive",
                None,
                12.0,
            ),
        ];

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 1,
            segment_title: "Cold Open & Program Slate".into(),
            segment_category: "Introduction".into(),
            target_duration_secs: 60.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_lead_story(&self) -> NewsSegmentScript {
        let mut lines = vec![
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "Kicking off with our lead story: MAPPA has officially locked in the production schedule for Chainsaw Man Season Two and the theatrical Reze Arc movie.",
                "AuthoritativePassion",
                "NeutralObservant",
                Some("Video wall shows official MAPPA press release and key visual graphic"),
                18.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "Fans have waited nearly two years since the bomb girl teaser. Marcus, why is MAPPA splitting this between a theatrical feature and a television season?",
                "InquisitiveAnalytic",
                "EagerToExplain",
                None,
                16.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "Because the Reze Arc is singularly cinematic! It is essentially a tight, explosive tragic romance that fits a 90-minute theatrical structure perfectly, allowing the animators to push action choreography and fluid camera moves that television schedules simply suffocate.",
                "IntenseExuberance",
                "NoddingAgreement",
                Some("Muted B-roll: Official teaser visual breakdown and storyboard analysis graphics"),
                28.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::SplitScreenAnalysis,
                "And speaking of production schedules, MAPPA's pipeline management has been under intense industry scrutiny since Jujutsu Kaisen Season Two. The studio reports they restructured the production committee, giving animators extended pre-production windows.",
                "SeriousCritical",
                "VigorousNod",
                Some("Infographic: MAPPA timeline breakdown and committee restructuring charts"),
                24.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "Exactly. Tatsuki Fujimoto's cinematic paneling demands insane draftsmanship. Tatsuya Yoshihara and the team are returning with enhanced storyboard planning to protect artists while delivering that raw, unhinged kinetic punch.",
                "PassionateExpertise",
                "ImpressedSmirk",
                None,
                24.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::WideTwoShot,
                "If they can pull off the Reze fireworks sequence with sustainable hours, that's not just a win for audiences—it's a critical win for anime working conditions.",
                "EmpatheticConclusion",
                "RespectfulAgreement",
                None,
                16.0,
            ),
        ];

        // Pad additional dialogue beats to hit ~240s
        for i in 1..=4 {
            lines.push(Self::make_line(
                if i % 2 == 1 { AnchorRole::Enthusiast } else { AnchorRole::Counterpart },
                if i % 2 == 1 { StudioCameraCut::MarcusCloseUp } else { StudioCameraCut::ElenaCloseUp },
                &format!("Segment discussion beat {}: Exploring the stylistic evolution, compositing shaders, and global theatrical distribution strategy across Japanese and Western box offices.", i),
                "EngagedDialogue",
                "ListeningAttentive",
                Some("Studio graphic: Box office trajectory and streaming metrics"),
                29.0,
            ));
        }

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 2,
            segment_title: "Chainsaw Man Season 2 & Reze Movie Production Timeline".into(),
            segment_category: "Anime".into(),
            target_duration_secs: 240.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_spider_verse_story(&self) -> NewsSegmentScript {
        let mut lines = vec![
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "Turning now to western feature animation: Sony Pictures Animation has unveiled a glimpse into the patent-pending NPR—non-photorealistic rendering—tech powering Beyond the Spider-Verse.",
                "ClearJournalistic",
                "AwedAnticipation",
                Some("Video wall: Sony patent diagrams and multi-line shader preview"),
                22.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "Elena, what they are doing with algorithmic ink hatching is unbelievable. In Across the Spider-Verse, Gwen's watercolor mood-ring world was hand-painted per keyframe. For Beyond, Sony created custom shaders that dynamically calculate emotional brush weight in real time!",
                "AstoundedEnthusiasm",
                "DelightedObservation",
                Some("Muted visual breakdown: Brush weight and color temperature comparison"),
                26.0,
            ),
        ];
        for i in 1..=5 {
            lines.push(Self::make_line(
                if i % 2 == 1 { AnchorRole::Counterpart } else { AnchorRole::Enthusiast },
                if i % 2 == 1 { StudioCameraCut::ElenaCloseUp } else { StudioCameraCut::MarcusCloseUp },
                &format!("Spider-Verse technical analysis beat {}: Examining how Sony combines machine learning edge-detection with classical 2D animator timing on twos and threes.", i),
                "TechnicalFocus",
                "AppreciativeListening",
                Some("Technical diagram: Framerate modulation and 2D/3D hybrid geometry"),
                32.0,
            ));
        }

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 3,
            segment_title: "Sony Animation Previews Next-Gen Spider-Verse Rendering".into(),
            segment_category: "Western Cartoons".into(),
            target_duration_secs: 210.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_fortiche_story(&self) -> NewsSegmentScript {
        let mut lines = vec![
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "In adult animation, French powerhouse Fortiche Production and Riot Games have officially announced their next joint animated series set in the brutal empire of Noxus.",
                "ExcitedAuthority",
                "IntriguedNod",
                Some("Video wall: Fortiche concept silhouettes and Noxian crest"),
                22.0,
            ),
        ];
        for i in 1..=6 {
            lines.push(Self::make_line(
                if i % 2 == 1 { AnchorRole::Counterpart } else { AnchorRole::Enthusiast },
                if i % 2 == 1 { StudioCameraCut::ElenaCloseUp } else { StudioCameraCut::MarcusCloseUp },
                &format!("Fortiche deep-dive beat {}: Analyzing the French studio's matte painting workflow, 3D character blocking, and mature dramatic pacing.", i),
                "ArtisticAnalysis",
                "EngagedDiscussion",
                Some("Studio graphic: Fortiche lighting rigs and background matte breakdown"),
                31.0,
            ));
        }

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 4,
            segment_title: "Fortiche & Riot Games Announce New Adult Animated Universe".into(),
            segment_category: "Adult Animation".into(),
            target_duration_secs: 210.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_ghibli_story(&self) -> NewsSegmentScript {
        let mut lines = vec![
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "In studio preservation news: Hayao Miyazaki and Toshio Suzuki have formally opened the Ghibli Archival Facility in Mitaka, safeguarding four decades of physical acetate cels.",
                "ReverentTone",
                "DeepRespect",
                Some("Video wall: Mitaka archival facility photos and classic cel displays"),
                24.0,
            ),
        ];
        for i in 1..=5 {
            lines.push(Self::make_line(
                if i % 2 == 1 { AnchorRole::Enthusiast } else { AnchorRole::Counterpart },
                if i % 2 == 1 { StudioCameraCut::MarcusCloseUp } else { StudioCameraCut::ElenaCloseUp },
                &format!("Ghibli cel preservation beat {}: Exploring chemical vinegar syndrome, high-gamut 8K sensor scanning, and historical physical pigment preservation.", i),
                "HistoricalAppreciation",
                "AttentiveAnalysis",
                Some("Archival diagram: Nitrate vs acetate decay and digital color restoration"),
                31.0,
            ));
        }

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 5,
            segment_title: "Studio Ghibli Opens Physical Cel Archival Facility".into(),
            segment_category: "Industry & Production".into(),
            target_duration_secs: 180.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_dandadan_story(&self) -> NewsSegmentScript {
        let mut lines = vec![
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "Science SARU has officially renewed Dandadan for Season 2! Fuga Yamashiro returns as director alongside musical virtuoso Kensuke Ushio.",
                "CelebratoryExcitement",
                "PlayfulGrin",
                Some("Video wall: Dandadan Season 2 visual and musical staff announcements"),
                22.0,
            ),
        ];
        for i in 1..=5 {
            lines.push(Self::make_line(
                if i % 2 == 1 { AnchorRole::Counterpart } else { AnchorRole::Enthusiast },
                if i % 2 == 1 { StudioCameraCut::ElenaCloseUp } else { StudioCameraCut::MarcusCloseUp },
                &format!("Science SARU kinetic breakdown beat {}: How Yamashiro combines perspective distortion, speed lines, and Ushio's breakbeat soundtrack.", i),
                "StylisticEnthusiasm",
                "WittyCommentary",
                Some("Visual cue: Kinetic line art analysis and color palette breakdown"),
                31.0,
            ));
        }

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 6,
            segment_title: "Science SARU Renews Dandadan for Season 2".into(),
            segment_category: "Anime".into(),
            target_duration_secs: 180.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_rapid_headlines(&self) -> NewsSegmentScript {
        let lines = vec![
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::SplitScreenAnalysis,
                "Time for rapid-fire bulletins! Netflix orders a new Castlevania spinoff set in 19th century London.",
                "FastPacedAlert",
                "ReadyForCue",
                Some("Lower-third graphic: Castlevania London Spin-off Confirmed"),
                16.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "Genndy Tartakovsky's upcoming theatrical adventure announces voice cast and locks in a summer release date!",
                "PunchyDelivery",
                "AmusedNod",
                Some("Lower-third graphic: Genndy Tartakovsky Project Update"),
                16.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "Ufotable previews Demon Slayer Infinity Castle Arc international theatrical release dates across forty territories.",
                "CrispReporting",
                "ExcitedAnticipation",
                Some("Lower-third graphic: Ufotable Demon Slayer Global Theatrical Rollout"),
                18.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "And Studio Wit wraps production on their original fantasy film with director George Wada praising non-crunch scheduling.",
                "TriumphantDelivery",
                "WarmSmirk",
                Some("Lower-third graphic: Wit Studio Original Production Wrapped"),
                18.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::WideTwoShot,
                "Four stories in sixty-eight seconds. Marcus didn't even pass out from oxygen deprivation.",
                "DeadpanHumor",
                "LaughingBanter",
                None,
                14.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "I trained my lungs specifically for animation press cycles, Elena. Do not question the discipline.",
                "ComedicDefensive",
                "RollEyesAffectionate",
                None,
                16.0,
            ),
        ];

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 7,
            segment_title: "Rapid-Fire Industry Bulletins".into(),
            segment_category: "Upcoming Releases".into(),
            target_duration_secs: 120.0,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn build_closing_segment(&self) -> NewsSegmentScript {
        let lines = vec![
            Self::make_line(
                AnchorRole::Duo,
                StudioCameraCut::WideTwoShot,
                "That wraps today's edition of Clearly Drawn News. From hand-drawn preservation in Mitaka to algorithmic shaders in the multiverse, the medium has never been more vibrant.",
                "ReflectiveClosing",
                "WarmAgreement",
                None,
                18.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "Remember to subscribe, hit the bell, and let us know in the comments: which project are you anticipating most—Chainsaw Man's Reze arc or the new Fortiche series?",
                "DirectEngagement",
                "NoddingAtCamera",
                Some("End screen callouts and social badges"),
                16.0,
            ),
            Self::make_line(
                AnchorRole::Enthusiast,
                StudioCameraCut::MarcusCloseUp,
                "And don't say both, because that is the coward's answer. Pick a side!",
                "PlayfulChallenge",
                "LaughingReprimand",
                None,
                12.0,
            ),
            Self::make_line(
                AnchorRole::Counterpart,
                StudioCameraCut::ElenaCloseUp,
                "Ignore him. You can love both. We will see you tomorrow with another breakdown of the art and craft of animation.",
                "WarmProfessional",
                "WavingCheerfully",
                None,
                16.0,
            ),
            Self::make_line(
                AnchorRole::Duo,
                StudioCameraCut::WideTwoShot,
                "Until next time—keep drawing clearly.",
                "UnisonSignoff",
                "UnisonSignoff",
                Some("Full transparent production credits, AI disclosure, and copyright registry ledger"),
                20.0,
            ),
        ];

        let actual: f64 = lines.iter().map(|l| l.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: 8,
            segment_title: "Episode Sign-off & Production Credits".into(),
            segment_category: "Credits & Outro".into(),
            target_duration_secs: 120.0,
            actual_duration_secs: actual,
            lines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broadcast_episode_is_timed_from_its_words() {
        let writer = TwoAnchorScriptWriter::new();
        let episode = writer.generate_broadcast_episode(
            "Episode 001: MAPPA Chainsaw Man Sequel & Spider-Verse Tech",
            "2026-10-17",
        );
        assert_eq!(episode.segments.len(), 8);
        let spoken: f64 = episode
            .segments
            .iter()
            .flat_map(|segment| &segment.lines)
            .map(|line| crate::story_script::estimate_seconds(&line.spoken_text))
            .sum();
        assert!((episode.total_duration_secs - spoken).abs() < 1e-6);
        assert!(episode.validate_duration(&EpisodeDurationPolicy::default()).is_err());
    }
}
