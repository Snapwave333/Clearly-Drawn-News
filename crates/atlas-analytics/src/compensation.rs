use atlas_core::{AtlasError, AtlasResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonetizationEvidence {
    pub platform: String,
    pub period: String,
    pub real_revenue_usd_micros: u64,
    pub source_file: String,
    pub source_sha256: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimulatedPayrollRun {
    pub run_id: String,
    pub evidence: MonetizationEvidence,
    pub payouts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SimulatedPayrollLedger {
    pub runs: Vec<SimulatedPayrollRun>,
    pub agent_balances_micros: BTreeMap<String, u64>,
}

impl SimulatedPayrollLedger {
    pub fn load(path: &Path) -> AtlasResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
            AtlasError::Config(format!("Invalid simulated payroll ledger: {error}"))
        })
    }

    pub fn save(&self, path: &Path) -> AtlasResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn record_revenue(
        &mut self,
        platform: &str,
        period: &str,
        real_revenue_usd_micros: u64,
        source_file: &Path,
        agents: &[String],
    ) -> AtlasResult<&SimulatedPayrollRun> {
        let platform = platform.to_ascii_lowercase();
        if !matches!(platform.as_str(), "youtube" | "tiktok") {
            return Err(AtlasError::Config(
                "Payroll platform must be youtube or tiktok".into(),
            ));
        }
        if period.trim().is_empty() || agents.is_empty() {
            return Err(AtlasError::Config(
                "Payroll period and agent registry are required".into(),
            ));
        }
        if self
            .runs
            .iter()
            .any(|run| run.evidence.platform == platform && run.evidence.period == period)
        {
            return Err(AtlasError::Config(format!(
                "Revenue for {platform} period {period} was already paid"
            )));
        }
        let bytes = std::fs::read(source_file)?;
        if bytes.is_empty() {
            return Err(AtlasError::Config(
                "Monetization evidence file is empty".into(),
            ));
        }
        let source_sha256 = hex::encode(Sha256::digest(&bytes));
        let base = real_revenue_usd_micros / agents.len() as u64;
        let remainder = real_revenue_usd_micros % agents.len() as u64;
        let mut payouts = BTreeMap::new();
        for (index, agent) in agents.iter().enumerate() {
            let payout = base + u64::from((index as u64) < remainder);
            payouts.insert(agent.clone(), payout);
            *self.agent_balances_micros.entry(agent.clone()).or_default() += payout;
        }
        self.runs.push(SimulatedPayrollRun {
            run_id: format!("pay_{}_{period}", platform),
            evidence: MonetizationEvidence {
                platform,
                period: period.into(),
                real_revenue_usd_micros,
                source_file: source_file.to_string_lossy().into_owned(),
                source_sha256,
                recorded_at: Utc::now(),
            },
            payouts,
        });
        Ok(self.runs.last().expect("payroll run was just inserted"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revenue_increases_simulated_agent_balances() {
        let dir = tempfile::tempdir().unwrap();
        let evidence = dir.path().join("revenue.csv");
        std::fs::write(&evidence, "period,revenue\n2026-10,12.00").unwrap();
        let mut ledger = SimulatedPayrollLedger::default();
        ledger
            .record_revenue(
                "youtube",
                "2026-10",
                12_000_000,
                &evidence,
                &["writer".into(), "animator".into()],
            )
            .unwrap();
        assert_eq!(ledger.agent_balances_micros["writer"], 6_000_000);
        assert_eq!(ledger.agent_balances_micros["animator"], 6_000_000);
    }

    #[test]
    fn refuses_duplicate_period_payment() {
        let dir = tempfile::tempdir().unwrap();
        let evidence = dir.path().join("revenue.csv");
        std::fs::write(&evidence, "revenue").unwrap();
        let mut ledger = SimulatedPayrollLedger::default();
        let agents = vec!["writer".into()];
        ledger
            .record_revenue("tiktok", "2026-10", 1_000_000, &evidence, &agents)
            .unwrap();
        assert!(
            ledger
                .record_revenue("tiktok", "2026-10", 1_000_000, &evidence, &agents)
                .is_err()
        );
    }
}
