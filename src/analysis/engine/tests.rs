use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::*;
use crate::analysis::model::InputParticipant;

struct FakeClient {
    batches: Mutex<Vec<usize>>,
    repairs: AtomicUsize,
    fail_first_relationship: AtomicBool,
    fail_repair: bool,
}

impl FakeClient {
    fn new(fail_first_relationship: bool, fail_repair: bool) -> Self {
        Self {
            batches: Mutex::new(Vec::new()),
            repairs: AtomicUsize::new(0),
            fail_first_relationship: AtomicBool::new(fail_first_relationship),
            fail_repair,
        }
    }

    fn response(&self, messages: &[Message]) -> String {
        let input: Value = serde_json::from_str(&messages[1].content).unwrap();
        let repairing = messages.len() == 4;
        if repairing {
            self.repairs.fetch_add(1, Ordering::SeqCst);
            assert!(messages[3].content.contains("required_id"));
        }
        if let Some(participant) = input.get("participant") {
            let mut participant_id = participant["participant_id"].as_str().unwrap();
            let fail = if repairing {
                self.fail_repair
            } else {
                self.fail_first_relationship.swap(false, Ordering::SeqCst)
            };
            if fail {
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
    fn generate(&self, inputs: &[Vec<Message>]) -> Result<Vec<String>> {
        self.batches.lock().unwrap().push(inputs.len());
        Ok(inputs.iter().map(|input| self.response(input)).collect())
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

fn input(index: usize, participants: usize) -> InputInteraction {
    InputInteraction {
        interaction_id: format!("database-id-{index}"),
        channel: "imessage".into(),
        occurred_at: "2026-01-01".into(),
        direction: Some("incoming".into()),
        subject: None,
        body: "hello".into(),
        participants: (0..participants)
            .map(|participant| InputParticipant {
                participant_id: format!("person-{index}-{participant}"),
                display_name: format!("Person {index}-{participant}"),
                role: "sender".into(),
            })
            .collect(),
    }
}

#[test]
fn repairs_only_the_invalid_relationship_once() {
    let analyzer = Analyzer::from_client(FakeClient::new(true, false)).unwrap();

    let mut output = analyzer.analyze(&[input(0, 1)]).unwrap();
    let output = output.remove(0).unwrap();

    assert_eq!(
        output.items[0].relationship_signals[0].participant_id,
        "person-0-0"
    );
    assert_eq!(analyzer.client.repairs.load(Ordering::SeqCst), 1);
}

#[test]
fn stops_after_one_failed_repair() {
    let analyzer = Analyzer::from_client(FakeClient::new(true, true)).unwrap();

    let mut output = analyzer.analyze(&[input(0, 1)]).unwrap();
    let error = output.remove(0).unwrap_err();

    assert!(error.to_string().contains("failed after one repair"));
    assert_eq!(analyzer.client.repairs.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_content_and_relationships_across_interactions() {
    let analyzer = Analyzer::from_client(FakeClient::new(false, false)).unwrap();
    let inputs: Vec<_> = (0..8).map(|index| input(index, 2)).collect();

    let output = analyzer.analyze(&inputs).unwrap();

    assert!(output.into_iter().all(|item| item.is_ok()));
    assert_eq!(*analyzer.client.batches.lock().unwrap(), [8, 16]);
}
