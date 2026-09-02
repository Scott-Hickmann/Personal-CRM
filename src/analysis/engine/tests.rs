use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use super::*;
use crate::analysis::model::InputParticipant;

struct FakeClient {
    active: AtomicUsize,
    maximum: AtomicUsize,
    repairs: AtomicUsize,
    fail_first_relationship: AtomicBool,
    fail_repair: bool,
}

impl FakeClient {
    fn new(fail_first_relationship: bool, fail_repair: bool) -> Self {
        Self {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            repairs: AtomicUsize::new(0),
            fail_first_relationship: AtomicBool::new(fail_first_relationship),
            fail_repair,
        }
    }

    fn response<T: Serialize>(&self, input: &T, allow_failure: bool) -> String {
        let input = serde_json::to_value(input).unwrap();
        if let Some(participant) = input.get("participant") {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);
            let mut participant_id = participant["participant_id"].as_str().unwrap();
            if allow_failure && self.fail_first_relationship.swap(false, Ordering::SeqCst) {
                participant_id = "participant-wrong";
            }
            relationship(participant_id)
        } else {
            json!({
                "interaction_id": input["interaction_id"],
                "summary": "hello",
                "is_personal": true,
                "mentions": []
            })
            .to_string()
        }
    }
}

impl ModelClient for FakeClient {
    fn generate<T: Serialize>(&self, _prompt: &str, _schema: &Value, input: &T) -> Result<String> {
        Ok(self.response(input, true))
    }

    fn repair<T: Serialize>(
        &self,
        _prompt: &str,
        _schema: &Value,
        input: &T,
        _invalid: &str,
        repair: &str,
    ) -> Result<String> {
        self.repairs.fetch_add(1, Ordering::SeqCst);
        assert!(repair.contains("required_id"));
        if self.fail_repair {
            Ok(relationship("participant-wrong"))
        } else {
            Ok(self.response(input, false))
        }
    }
}

fn relationship(participant_id: &str) -> String {
    json!({
        "participant_id": participant_id,
        "intimacy": 0,
        "emotional_support": 0,
        "practical_support": 0,
        "affection": 0,
        "shared_activity": 0,
        "conflict_repair": 0,
        "confidence": 1,
        "evidence": ""
    })
    .to_string()
}

fn input(participants: usize) -> InputInteraction {
    InputInteraction {
        interaction_id: "database-id".into(),
        channel: "imessage".into(),
        occurred_at: "2026-01-01".into(),
        direction: Some("incoming".into()),
        subject: None,
        body: "hello".into(),
        participants: (0..participants)
            .map(|index| InputParticipant {
                participant_id: format!("person-{index}"),
                display_name: format!("Person {index}"),
                role: "sender".into(),
            })
            .collect(),
    }
}

#[test]
fn repairs_only_the_invalid_relationship_once() {
    let analyzer = Analyzer::from_client(FakeClient::new(true, false)).unwrap();

    let output = analyzer.analyze(&input(1)).unwrap();

    assert_eq!(
        output.items[0].relationship_signals[0].participant_id,
        "person-0"
    );
    assert_eq!(analyzer.client.repairs.load(Ordering::SeqCst), 1);
}

#[test]
fn stops_after_one_failed_repair() {
    let analyzer = Analyzer::from_client(FakeClient::new(true, true)).unwrap();

    let error = analyzer.analyze(&input(1)).unwrap_err();

    assert!(error.to_string().contains("failed after one repair"));
    assert_eq!(analyzer.client.repairs.load(Ordering::SeqCst), 1);
}

#[test]
fn runs_at_most_three_relationship_calls_together() {
    let analyzer = Analyzer::from_client(FakeClient::new(false, false)).unwrap();

    let output = analyzer.analyze(&input(7)).unwrap();

    assert_eq!(output.items[0].relationship_signals.len(), 7);
    assert_eq!(analyzer.client.maximum.load(Ordering::SeqCst), 3);
}
