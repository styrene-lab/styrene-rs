//! Native NomadNet request registration.

use std::sync::Arc;

use rns_core::destination::{RequestAccess, RequestRegistrationError, SingleInputDestination};
use tokio::sync::Mutex;

use crate::services::{NativePageEntry, PageService};

const MAX_REQUEST_SIZE: usize = 64 * 1024;
const MAX_RESPONSE_SIZE: usize = 32 * 1024 * 1024 + 5;
const EXECUTION_ERROR_PAGE: &[u8] = b">Page Error\n\nThe page could not be generated.\n";

pub async fn register_handlers(
    destination: Arc<Mutex<SingleInputDestination>>,
    pages: Arc<PageService>,
) -> Result<(), RequestRegistrationError> {
    pages.clear_active_native_paths();
    for entry in pages.native_entries() {
        let request_path = entry.request_path.clone();
        register_entry(&destination, Arc::clone(&pages), entry).await?;
        pages.mark_native_path_active(request_path);
    }
    Ok(())
}

async fn register_entry(
    destination: &Arc<Mutex<SingleInputDestination>>,
    pages: Arc<PageService>,
    entry: NativePageEntry,
) -> Result<(), RequestRegistrationError> {
    let access = entry
        .allowed_identities
        .clone()
        .map(RequestAccess::AllowList)
        .unwrap_or(RequestAccess::Public);
    let path = entry.request_path.clone();
    destination.lock().await.register_request_path(
        &path,
        access,
        MAX_REQUEST_SIZE,
        MAX_RESPONSE_SIZE,
        Arc::new(move |data, remote_identity, link, _| {
            let content = pages
                .serve_native(&entry, data, remote_identity, link.link_id)
                .unwrap_or_else(|| EXECUTION_ERROR_PAGE.to_vec());
            encode_native_response(&entry.request_path, &content)
        }),
    )?;
    Ok(())
}

/// Encode a native response the way NomadNet clients expect.
///
/// Pages are a single binary value. Files are a `[name, data]` pair: NomadNet
/// saves a file response from its request metadata when present and otherwise
/// from a two-element list, and this host does not attach response metadata.
fn encode_native_response(request_path: &str, content: &[u8]) -> Vec<u8> {
    match file_name_for_request(request_path) {
        Some(name) => encode_file_pair(name, content),
        None => encode_binary(content),
    }
}

fn file_name_for_request(request_path: &str) -> Option<&str> {
    let relative = request_path.strip_prefix("/file/")?;
    relative.rsplit('/').next().filter(|name| !name.is_empty())
}

fn encode_file_pair(name: &str, content: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(content.len() + name.len() + 8);
    let value = rmpv::Value::Array(vec![
        rmpv::Value::String(name.into()),
        rmpv::Value::Binary(content.to_vec()),
    ]);
    if rmpv::encode::write_value(&mut encoded, &value).is_err() {
        return Vec::new();
    }
    encoded
}

fn encode_binary(content: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(content.len() + 5);
    if rmpv::encode::write_value(&mut encoded, &rmpv::Value::Binary(content.to_vec())).is_err() {
        return Vec::new();
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use rns_core::destination::{
        DestinationName, RequestDispatchError, RequestLinkContext, SingleInputDestination,
        request_path_hash,
    };
    use rns_core::hash::AddressHash;
    use rns_core::identity::PrivateIdentity;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use std::time::Duration;

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    fn request_data(entries: &[(&str, &str)]) -> Vec<u8> {
        let value = rmpv::Value::Map(
            entries
                .iter()
                .map(|(name, value)| (rmpv::Value::from(*name), rmpv::Value::from(*value)))
                .collect(),
        );
        let mut encoded = Vec::new();
        rmpv::encode::write_value(&mut encoded, &value).unwrap();
        encoded
    }

    #[cfg(unix)]
    fn test_page_service(
        pages_dir: std::path::PathBuf,
        files_dir: std::path::PathBuf,
    ) -> PageService {
        PageService::with_execution_limits(
            pages_dir,
            files_dir,
            Duration::from_secs(60),
            1024 * 1024,
        )
    }

    #[cfg(unix)]
    async fn dispatch_raw(
        pages: Arc<PageService>,
        path: &str,
        data: &[u8],
        remote: Option<&rns_core::identity::Identity>,
    ) -> Result<Vec<u8>, RequestDispatchError> {
        let destination = Arc::new(Mutex::new(SingleInputDestination::new(
            PrivateIdentity::new_from_name("dynamic-native-host"),
            DestinationName::new("nomadnetwork", "node"),
        )));
        let entry =
            pages.native_entries().into_iter().find(|entry| entry.request_path == path).unwrap();
        register_entry(&destination, pages, entry).await.unwrap();
        let destination_hash = destination.lock().await.desc.address_hash;

        destination.lock().await.dispatch_request(
            &request_path_hash(path),
            data,
            remote,
            &RequestLinkContext {
                link_id: AddressHash::new([0xab; 16]),
                destination: destination_hash,
            },
            [0xcd; 16],
        )
    }

    #[cfg(unix)]
    async fn dispatch_dynamic(
        pages: Arc<PageService>,
        path: &str,
        data: &[u8],
        remote: Option<&rns_core::identity::Identity>,
    ) -> Vec<u8> {
        let response = dispatch_raw(pages, path, data, remote).await.unwrap();
        match rmpv::decode::read_value(&mut response.as_slice()).unwrap() {
            rmpv::Value::Binary(bytes) => bytes,
            other => panic!("unexpected dynamic response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn registration_serves_page_and_file_content() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let pages_dir = root_path.join("pages");
        let files_dir = root_path.join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(pages_dir.join("small.mu"), b"small").unwrap();
        std::fs::write(files_dir.join("small.bin"), b"file").unwrap();
        let pages = Arc::new(PageService::with_storage_dirs(pages_dir, files_dir));
        let identity = PrivateIdentity::new_from_name("native-host-registration");
        let destination = Arc::new(Mutex::new(SingleInputDestination::new(
            identity,
            DestinationName::new("nomadnetwork", "node"),
        )));
        register_handlers(Arc::clone(&destination), Arc::clone(&pages)).await.unwrap();
        assert!(pages.native_inventory().iter().all(|(_, active)| *active));
        let destination_hash = destination.lock().await.desc.address_hash;
        let context = RequestLinkContext {
            link_id: AddressHash::new([1; 16]),
            destination: destination_hash,
        };

        for (path, request_id, expected) in [
            ("/page/small.mu", [2; 16], rmpv::Value::Binary(b"small".to_vec())),
            (
                "/file/small.bin",
                [3; 16],
                rmpv::Value::Array(vec![
                    rmpv::Value::String("small.bin".into()),
                    rmpv::Value::Binary(b"file".to_vec()),
                ]),
            ),
        ] {
            let response = destination
                .lock()
                .await
                .dispatch_request(&request_path_hash(path), &[0xc0], None, &context, request_id)
                .unwrap();
            assert_eq!(rmpv::decode::read_value(&mut response.as_slice()).unwrap(), expected);
        }
    }

    #[test]
    fn file_responses_use_the_nomadnet_name_and_data_pair() {
        assert_eq!(file_name_for_request("/file/docs/manual.bin"), Some("manual.bin"));
        assert_eq!(file_name_for_request("/file/"), None);
        assert_eq!(file_name_for_request("/page/index.mu"), None);
        let encoded = encode_native_response("/file/manual.bin", b"payload");
        assert_eq!(
            rmpv::decode::read_value(&mut encoded.as_slice()).unwrap(),
            rmpv::Value::Array(vec![
                rmpv::Value::String("manual.bin".into()),
                rmpv::Value::Binary(b"payload".to_vec()),
            ])
        );
        let page = encode_native_response("/page/index.mu", b"page");
        assert_eq!(
            rmpv::decode::read_value(&mut page.as_slice()).unwrap(),
            rmpv::Value::Binary(b"page".to_vec())
        );
    }

    #[tokio::test]
    async fn allowed_policy_requires_authenticated_listed_identity_before_handler() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let pages_dir = root_path.join("pages");
        let files_dir = root_path.join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let allowed = PrivateIdentity::new_from_name("allowed-native-reader");
        std::fs::write(pages_dir.join("private.mu"), b"secret").unwrap();
        std::fs::write(
            pages_dir.join("private.mu.allowed"),
            hex::encode(allowed.address_hash().as_slice()),
        )
        .unwrap();
        let pages = Arc::new(PageService::with_storage_dirs(pages_dir, files_dir));
        let destination = Arc::new(Mutex::new(SingleInputDestination::new(
            PrivateIdentity::new_from_name("authorized-native-host"),
            DestinationName::new("nomadnetwork", "node"),
        )));
        let entry = pages
            .native_entries()
            .into_iter()
            .find(|entry| entry.request_path == "/page/private.mu")
            .unwrap();
        register_entry(&destination, pages, entry.clone()).await.unwrap();
        let destination_hash = destination.lock().await.desc.address_hash;
        let context = RequestLinkContext {
            link_id: AddressHash::new([4; 16]),
            destination: destination_hash,
        };
        let path = request_path_hash(&entry.request_path);

        assert_eq!(
            destination.lock().await.dispatch_request(&path, &[0xc0], None, &context, [5; 16]),
            Err(RequestDispatchError::Unauthorized)
        );
        assert!(
            destination
                .lock()
                .await
                .dispatch_request(&path, &[0xc0], Some(allowed.as_identity()), &context, [6; 16])
                .is_ok()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dynamic_page_receives_nomadnet_fields_identity_and_reduced_environment() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let expected_path = std::env::var("PATH").unwrap();
        write_script(
            &pages_dir.join("fields.mu"),
            r#"printf '%s|%s|%s|%s|%s|%s|%s' "$field_name" "$var_mode" "${ignored-}" "$link_id" "$remote_identity" "$PWD" "$PATH"
test -z "${HOME-}"
test -z "${STYRENE_PAGE_TEST_SECRET-}""#,
        );
        let pages = Arc::new(test_page_service(pages_dir.clone(), files_dir));
        let remote = PrivateIdentity::new_from_name("dynamic-reader");
        let response = dispatch_dynamic(
            pages,
            "/page/fields.mu",
            &request_data(&[("field_name", "Ada"), ("var_mode", "safe"), ("ignored", "no")]),
            Some(remote.as_identity()),
        )
        .await;
        let text = String::from_utf8(response).unwrap();
        let parts = text.split('|').collect::<Vec<_>>();
        assert_eq!(parts[0..3], ["Ada", "safe", ""]);
        assert_eq!(parts[3], "ab".repeat(16));
        assert_eq!(parts[4], hex::encode(remote.address_hash().as_slice()));
        assert!(!parts[5].starts_with(&pages_dir.to_string_lossy().to_string()));
        assert_eq!(parts[6], expected_path);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_dynamic_page_returns_without_deadline_and_cleans_descendants() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let descendant_pid = root.path().join("success-descendant-pid");
        write_script(
            &pages_dir.join("success.mu"),
            &format!(
                "/bin/sleep 60 >/dev/null 2>&1 &\nprintf '%s' \"$!\" > '{}'\nprintf success",
                descendant_pid.display()
            ),
        );
        let pages = Arc::new(PageService::with_execution_limits(
            pages_dir,
            files_dir,
            Duration::from_secs(5),
            1024,
        ));
        let response = dispatch_dynamic(pages, "/page/success.mu", &[0xc0], None).await;
        assert_eq!(response, b"success");
        let descendant_pid =
            std::fs::read_to_string(descendant_pid).unwrap().parse::<u32>().unwrap().to_string();
        let process = std::process::Command::new("/bin/ps")
            .args(["-p", &descendant_pid, "-o", "pid="])
            .output()
            .unwrap();
        assert!(process.stdout.is_empty(), "dynamic page descendant still exists");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn authorization_is_checked_before_dynamic_execution() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let marker = root.path().join("executed");
        write_script(
            &pages_dir.join("private.mu"),
            &format!("/usr/bin/touch '{}'\nprintf allowed", marker.display()),
        );
        let allowed = PrivateIdentity::new_from_name("allowed-dynamic-reader");
        std::fs::write(
            pages_dir.join("private.mu.allowed"),
            hex::encode(allowed.address_hash().as_slice()),
        )
        .unwrap();
        let pages = Arc::new(test_page_service(pages_dir, files_dir));
        let destination = Arc::new(Mutex::new(SingleInputDestination::new(
            PrivateIdentity::new_from_name("private-dynamic-host"),
            DestinationName::new("nomadnetwork", "node"),
        )));
        let entry = pages
            .native_entries()
            .into_iter()
            .find(|entry| entry.request_path == "/page/private.mu")
            .unwrap();
        register_entry(&destination, pages, entry).await.unwrap();
        let destination_hash = destination.lock().await.desc.address_hash;
        let result = destination.lock().await.dispatch_request(
            &request_path_hash("/page/private.mu"),
            &[0xc0],
            None,
            &RequestLinkContext {
                link_id: AddressHash::new([1; 16]),
                destination: destination_hash,
            },
            [2; 16],
        );
        assert_eq!(result, Err(RequestDispatchError::Unauthorized));
        assert!(!marker.exists());

        let response = destination
            .lock()
            .await
            .dispatch_request(
                &request_path_hash("/page/private.mu"),
                &[0xc0],
                Some(allowed.as_identity()),
                &RequestLinkContext {
                    link_id: AddressHash::new([1; 16]),
                    destination: destination_hash,
                },
                [3; 16],
            )
            .unwrap();
        assert_eq!(
            rmpv::decode::read_value(&mut response.as_slice()).unwrap(),
            rmpv::Value::Binary(b"allowed".to_vec())
        );
        assert!(marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dynamic_page_timeout_cleans_up_descendants() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let descendant_pid = root.path().join("timeout-descendant-pid");
        write_script(
            &pages_dir.join("timeout.mu"),
            &format!(
                "/bin/sleep 60 &\nprintf '%s' \"$!\" > '{}'\n/bin/sleep 10",
                descendant_pid.display()
            ),
        );
        let pages = Arc::new(PageService::with_execution_limits(
            pages_dir,
            files_dir,
            Duration::from_millis(250),
            1024,
        ));
        let response = dispatch_dynamic(pages, "/page/timeout.mu", &[0xc0], None).await;
        assert_eq!(response, EXECUTION_ERROR_PAGE);
        let descendant_pid =
            std::fs::read_to_string(descendant_pid).unwrap().parse::<u32>().unwrap().to_string();
        let process = std::process::Command::new("/bin/ps")
            .args(["-p", &descendant_pid, "-o", "pid="])
            .output()
            .unwrap();
        assert!(process.stdout.is_empty(), "timed-out dynamic page descendant still exists");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn escaped_session_descendant_is_tracked_and_removed() {
        if !Path::new("/usr/bin/perl").is_file() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let marker = root.path().join("escaped-session-survived");
        write_script(
            &pages_dir.join("escape.mu"),
            r#"/usr/bin/perl -MPOSIX -e 'POSIX::setsid(); select undef, undef, undef, 0.8; open my $fh, ">", $ENV{var_marker}; print $fh "escaped"' >/dev/null 2>&1 &
/bin/sleep 0.2
printf escaped"#,
        );
        let pages = Arc::new(PageService::with_execution_limits(
            pages_dir,
            files_dir,
            Duration::from_secs(60),
            1024,
        ));
        let response = dispatch_dynamic(
            pages,
            "/page/escape.mu",
            &request_data(&[("var_marker", &marker.to_string_lossy())]),
            None,
        )
        .await;
        assert_eq!(response, b"escaped");
        std::thread::sleep(Duration::from_secs(1));
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dynamic_page_output_and_errors_are_bounded_and_redacted() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let overflow_marker = root.path().join("overflow-descendant-survived");
        let error_marker = root.path().join("error-descendant-survived");
        write_script(
            &pages_dir.join("limit.mu"),
            &format!(
                "( /bin/sleep 0.4; /usr/bin/touch '{}' ) >/dev/null 2>&1 &\nwhile :; do printf 0123456789abcdef; done",
                overflow_marker.display()
            ),
        );
        write_script(
            &pages_dir.join("error.mu"),
            &format!(
                "( /bin/sleep 0.4; /usr/bin/touch '{}' ) >/dev/null 2>&1 &\nprintf '%s' \"$field_password\" >&2\nexit 7",
                error_marker.display()
            ),
        );
        let pages = Arc::new(PageService::with_execution_limits(
            pages_dir,
            files_dir,
            Duration::from_secs(2),
            128,
        ));
        let limited = dispatch_dynamic(Arc::clone(&pages), "/page/limit.mu", &[0xc0], None).await;
        assert_eq!(limited, EXECUTION_ERROR_PAGE);
        let secret = "not-in-response-or-debug";
        let failed = dispatch_dynamic(
            pages,
            "/page/error.mu",
            &request_data(&[("field_password", secret)]),
            None,
        )
        .await;
        assert_eq!(failed, EXECUTION_ERROR_PAGE);
        assert!(!String::from_utf8_lossy(&failed).contains(secret));
        std::thread::sleep(Duration::from_millis(500));
        assert!(!overflow_marker.exists());
        assert!(!error_marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn malformed_and_oversized_field_requests_fail_without_execution() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let marker = root.path().join("malformed-executed");
        write_script(
            &pages_dir.join("validate.mu"),
            &format!("/usr/bin/touch '{}'\nprintf executed", marker.display()),
        );
        let pages = Arc::new(test_page_service(pages_dir, files_dir));

        let mut malformed_values = vec![
            rmpv::Value::Array(Vec::new()),
            rmpv::Value::Map(vec![(rmpv::Value::from(7), rmpv::Value::from("value"))]),
            rmpv::Value::Map(vec![(rmpv::Value::from("field_count"), rmpv::Value::from(7))]),
            rmpv::Value::Map(vec![(
                rmpv::Value::from("field_bad=name"),
                rmpv::Value::from("value"),
            )]),
            rmpv::Value::Map(vec![(
                rmpv::Value::from("field_bad\0name"),
                rmpv::Value::from("value"),
            )]),
            rmpv::Value::Map(vec![(
                rmpv::Value::from("field_large"),
                rmpv::Value::from("x".repeat(16 * 1024 + 1)),
            )]),
            rmpv::Value::Map(vec![
                (rmpv::Value::from("field_duplicate"), rmpv::Value::from("one")),
                (rmpv::Value::from("field_duplicate"), rmpv::Value::from("two")),
            ]),
        ];
        malformed_values.push(rmpv::Value::Map(
            (0..65)
                .map(|index| {
                    (rmpv::Value::from(format!("field_{index}")), rmpv::Value::from("value"))
                })
                .collect(),
        ));
        for value in malformed_values {
            let mut encoded = Vec::new();
            rmpv::encode::write_value(&mut encoded, &value).unwrap();
            let response =
                dispatch_dynamic(Arc::clone(&pages), "/page/validate.mu", &encoded, None).await;
            assert_eq!(response, EXECUTION_ERROR_PAGE);
            assert!(!marker.exists());
        }
        let trailing = [0xc0, 0xc0];
        assert_eq!(
            dispatch_dynamic(Arc::clone(&pages), "/page/validate.mu", &trailing, None,).await,
            EXECUTION_ERROR_PAGE
        );
        assert!(!marker.exists());

        let oversized = vec![0_u8; MAX_REQUEST_SIZE + 1];
        assert_eq!(
            dispatch_raw(pages, "/page/validate.mu", &oversized, None).await,
            Err(RequestDispatchError::RequestTooLarge)
        );
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn submitted_shell_metacharacters_are_environment_data_only() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let marker = root.path().join("interpolated");
        write_script(&pages_dir.join("literal.mu"), "printf '%s' \"$field_literal\"");
        let value = format!("$(/usr/bin/touch '{}');`false`;*;$HOME", marker.display());
        let pages = Arc::new(test_page_service(pages_dir, files_dir));
        let response = dispatch_dynamic(
            pages,
            "/page/literal.mu",
            &request_data(&[("field_literal", &value)]),
            None,
        )
        .await;
        assert_eq!(response, value.as_bytes());
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn registered_executable_is_immutable_and_debug_redacted() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        let script = pages_dir.join("fixed.mu");
        write_script(
            &script,
            "printf original-secret-script; chmod u+w \"$0\"; printf '#!/bin/sh\\nprintf compromised\\n' > \"$0\"",
        );
        let pages = Arc::new(test_page_service(pages_dir, files_dir));
        let entry = pages
            .native_entries()
            .into_iter()
            .find(|entry| entry.request_path == "/page/fixed.mu")
            .unwrap();
        assert!(!format!("{entry:?}").contains("original-secret-script"));
        write_script(&script, "printf replacement");

        let destination = Arc::new(Mutex::new(SingleInputDestination::new(
            PrivateIdentity::new_from_name("fixed-dynamic-host"),
            DestinationName::new("nomadnetwork", "node"),
        )));
        register_entry(&destination, pages, entry).await.unwrap();
        let destination_hash = destination.lock().await.desc.address_hash;
        for request_id in [[4; 16], [5; 16]] {
            let response = destination
                .lock()
                .await
                .dispatch_request(
                    &request_path_hash("/page/fixed.mu"),
                    &[0xc0],
                    None,
                    &RequestLinkContext {
                        link_id: AddressHash::new([3; 16]),
                        destination: destination_hash,
                    },
                    request_id,
                )
                .unwrap();
            assert_eq!(
                rmpv::decode::read_value(&mut response.as_slice()).unwrap(),
                rmpv::Value::Binary(b"original-secret-script".to_vec())
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn password_is_absent_from_response_debug_and_captured_diagnostics() {
        let root = tempfile::tempdir().unwrap();
        let pages_dir = root.path().join("pages");
        let files_dir = root.path().join("files");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::create_dir_all(&files_dir).unwrap();
        write_script(&pages_dir.join("password.mu"), "printf '%s' \"$field_password\" >&2\nexit 9");
        let pages = Arc::new(test_page_service(pages_dir, files_dir));
        let entry = pages
            .native_entries()
            .into_iter()
            .find(|entry| entry.request_path == "/page/password.mu")
            .unwrap();
        let secret = "password-must-not-escape";
        assert!(!format!("{entry:?}").contains(secret));
        crate::diagnostics::start_capture();
        let response = dispatch_dynamic(
            pages,
            "/page/password.mu",
            &request_data(&[("field_password", secret)]),
            None,
        )
        .await;
        let diagnostics = crate::diagnostics::finish_capture().join("\n");
        assert!(!String::from_utf8_lossy(&response).contains(secret));
        assert!(!diagnostics.contains(secret));
        assert!(diagnostics.contains("dynamic page execution failed"));
    }
}

#[cfg(test)]
mod pinned_fixture_tests {
    //! Byte-level checks against `tests/interop/fixtures/nomadnet-v1`, which the
    //! pinned Python NomadNet node and RNS `Link.handle_request` produced.

    use super::encode_native_response;
    use rns_core::transport::request::encode_response_envelope;

    const STATIC_PAGE: &[u8] =
        include_bytes!("../../../../../tests/interop/fixtures/nomadnet-v1/response_index.bin");
    const PYTHON_ENVELOPE: &[u8] = include_bytes!(
        "../../../../../tests/interop/fixtures/nomadnet-v1/response_envelope_index.msgpack"
    );
    const INDEX: &str =
        include_str!("../../../../../tests/interop/fixtures/nomadnet-v1/index.json");

    #[test]
    fn native_page_response_matches_pinned_python_packet_envelope() {
        let index: serde_json::Value = serde_json::from_str(INDEX).expect("fixture index");
        let vector = index["vectors"]
            .as_array()
            .expect("vectors")
            .iter()
            .find(|vector| vector["id"] == "rns-response-envelope-index")
            .expect("response envelope vector");
        let request_id: [u8; 16] =
            hex::decode(vector["expected"]["request_id_hex"].as_str().expect("id"))
                .expect("hex")
                .try_into()
                .expect("16 bytes");
        let native = encode_native_response("/page/index.mu", STATIC_PAGE);
        let envelope = encode_response_envelope(request_id, &native).expect("envelope");
        assert_eq!(envelope, PYTHON_ENVELOPE);
    }
}
