//! Fixed offline project acceptance, not a general agent admission API.
use cocoon_guest_wire::{Identity, Operation, Output, Request, Response};
use serde_json::Value;
use uuid::Uuid;

const KNOWLEDGE: &str = "Cocoon\n";
const ORIGINAL: &str = "exit 1\n";
const SCRIPT: &str = "set -eu\nIFS= read -r name < knowledge.txt\nprintf 'Built for %s\\n' \"$name\" > artifact.txt\nIFS= read -r result < artifact.txt\n[ \"$result\" = 'Built for Cocoon' ]\nprintf 'TEST_PASS\\n'\n";
pub(crate) const ARTIFACT: &str = "Built for Cocoon\n";

pub(crate) fn requests(attempt: &str) -> Vec<Request> {
    let task = Uuid::new_v4().to_string();
    let operations = vec![
        Operation::Write {
            path: "/workspace/knowledge.txt".into(),
            content: KNOWLEDGE.into(),
        },
        Operation::Write {
            path: "/workspace/build.sh".into(),
            content: ORIGINAL.into(),
        },
        Operation::Read {
            path: "/workspace/build.sh".into(),
        },
        Operation::Write {
            path: "/workspace/build.sh".into(),
            content: SCRIPT.into(),
        },
        Operation::Exec {
            argv: vec!["/bin/sh".into(), "build.sh".into()],
            cwd: "/workspace".into(),
        },
        Operation::Read {
            path: "/workspace/artifact.txt".into(),
        },
        Operation::Exec {
            argv: vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf failure >&2; exit 7".into(),
            ],
            cwd: "/workspace".into(),
        },
        Operation::Exec {
            argv: vec!["/bin/sleep".into(), "5".into()],
            cwd: "/workspace".into(),
        },
    ];
    operations
        .into_iter()
        .enumerate()
        .map(|(i, operation)| Request {
            identity: Identity {
                vm: String::new(),
                caller: "local-project-acceptance".into(),
                task: task.clone(),
                attempt: attempt.into(),
                generation: 0,
                request: i as u32,
            },
            operation,
        })
        .collect()
}

pub(crate) fn verify(requests: &[Request], value: &Value) -> bool {
    let Ok(responses) = serde_json::from_value::<Vec<Response>>(value.clone()) else {
        return false;
    };
    if responses.len() != requests.len() || responses.len() != 8 {
        return false;
    }
    responses
        .iter()
        .zip(requests)
        .enumerate()
        .all(|(i, (response, request))| {
            if response.identity != request.identity {
                return false;
            }
            match (i, &response.result) {
                (0 | 1 | 3, Ok(Output::Written)) => true,
                (2, Ok(Output::Content { content })) => content == ORIGINAL,
                (
                    4,
                    Ok(Output::Exited {
                        code,
                        stdout,
                        stderr,
                        timed_out,
                    }),
                ) => *code == Some(0) && stdout == "TEST_PASS\n" && stderr.is_empty() && !timed_out,
                (5, Ok(Output::Content { content })) => content == ARTIFACT,
                (
                    6,
                    Ok(Output::Exited {
                        code,
                        stdout,
                        stderr,
                        timed_out,
                    }),
                ) => *code == Some(7) && stdout.is_empty() && stderr == "failure" && !timed_out,
                (
                    7,
                    Ok(Output::Exited {
                        code, timed_out, ..
                    }),
                ) => code.is_none() && *timed_out,
                _ => false,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_identity_and_content_are_not_trusted() {
        let requests = requests("attempt");
        for request in &requests {
            request.validate().unwrap();
        }
        assert!(!verify(&requests, &serde_json::json!([])));
        let results = vec![
            Output::Written,
            Output::Written,
            Output::Content {
                content: ORIGINAL.into(),
            },
            Output::Written,
            Output::Exited {
                code: Some(0),
                stdout: "TEST_PASS\n".into(),
                stderr: "".into(),
                timed_out: false,
            },
            Output::Content {
                content: ARTIFACT.into(),
            },
            Output::Exited {
                code: Some(7),
                stdout: "".into(),
                stderr: "failure".into(),
                timed_out: false,
            },
            Output::Exited {
                code: None,
                stdout: "".into(),
                stderr: "".into(),
                timed_out: true,
            },
        ];
        let responses: Vec<_> = requests
            .iter()
            .zip(results)
            .map(|(r, output)| Response {
                identity: r.identity.clone(),
                result: Ok(output),
            })
            .collect();
        let mut value = serde_json::to_value(responses).unwrap();
        assert!(verify(&requests, &value));
        value[5]["identity"]["attempt"] = "another-attempt".into();
        assert!(!verify(&requests, &value));
    }
}
