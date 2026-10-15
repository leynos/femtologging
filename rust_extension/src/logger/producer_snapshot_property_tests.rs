//! Property tests for producer-side per-handler record snapshots.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use proptest::prelude::*;

use super::super::logger_tests_helpers::CollectingHandler;
use super::{FemtoLogger, HandlerAttachment, TestFilter};
use crate::filters::FemtoFilter;
use crate::handler::FemtoHandlerTrait;
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;

proptest! {
    #[test]
    fn handler_filter_chains_match_the_ordered_snapshot_model(
        filter_chains in prop::collection::vec(
            prop::collection::vec((any::<bool>(), any::<u8>()), 0..5),
            1..6,
        )
    ) {
        let base_record = FemtoLogRecord::new("producer", FemtoLevel::Info, "property");
        let mut expected_record = base_record.clone();
        let mut record = base_record.clone();
        let mut attachments = Vec::with_capacity(filter_chains.len());
        let mut expected = Vec::new();

        for (handler_index, filter_chain) in filter_chains.iter().enumerate() {
            let collector = Arc::new(CollectingHandler::new());
            let handler: Arc<dyn FemtoHandlerTrait> = collector.clone();
            let filters = filter_chain
                .iter()
                .enumerate()
                .map(|(filter_index, (accepted, value))| {
                    Arc::new(TestFilter {
                        accepted: *accepted,
                        enrichment: BTreeMap::from([
                            (
                                format!("handler_{handler_index}_filter_{filter_index}"),
                                "applied".to_owned(),
                            ),
                            ("shared".to_owned(), value.to_string()),
                        ]),
                        calls: Arc::new(AtomicUsize::new(0)),
                    }) as Arc<dyn FemtoFilter>
                })
                .collect::<Vec<_>>();
            attachments.push(HandlerAttachment { handler, filters });

            let mut accepted_by_chain = true;
            for (filter_index, (accepted, value)) in filter_chain.iter().enumerate() {
                if !accepted {
                    accepted_by_chain = false;
                    break;
                }
                expected_record.metadata_mut().key_values.extend([
                    (
                        format!("handler_{handler_index}_filter_{filter_index}"),
                        "applied".to_owned(),
                    ),
                    ("shared".to_owned(), value.to_string()),
                ]);
            }
            if accepted_by_chain {
                expected.push((collector, expected_record.metadata().key_values.clone()));
            }
        }

        let handlers = FemtoLogger::filter_handlers_for_test(&mut record, attachments);
        prop_assert_eq!(handlers.len(), expected.len());
        for (handler, (collector, expected_fields)) in handlers.iter().zip(expected) {
            handler
                .handle(base_record.clone())
                .expect("snapshot handler should collect its record");
            let records = collector.collected();
            prop_assert_eq!(records.len(), 1);
            prop_assert_eq!(&records[0].metadata().key_values, &expected_fields);
        }
    }
}
