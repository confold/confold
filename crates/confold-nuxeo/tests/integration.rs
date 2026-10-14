//! Hermetic integration tests against a wiremock server speaking the documented Nuxeo REST shapes
//! (the tree mapping is specified in the crate docs). The synchronous `Source` calls run on a plain
//! std thread: the source owns its tokio runtime, and its `block_on` must not run inside one.

use confold_core::full_equal;
use confold_nuxeo::{NuxeoConfig, NuxeoSource};
use confold_vfs::{EntryKind, LocalSource, RelPath, Source};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Run a synchronous closure on its own thread (outside any async runtime).
fn run_sync<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f)
        .join()
        .expect("sync source thread panicked")
}

fn config(base_url: String) -> NuxeoConfig {
    NuxeoConfig {
        base_url,
        username: "nuxeo".to_owned(),
        password: "s3cret".to_owned(),
        root: "/ws".to_owned(),
    }
}

/// The `/ws` parent document (folderish).
fn ws_doc_mock() -> Mock {
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "document",
            "uid": "ws-1",
            "path": "/ws",
            "title": "Workspaces",
            "type": "Workspace",
            "state": "project",
            "lastModified": "2026-09-20T10:00:00.00Z",
            "facets": ["Folderish"],
            "properties": {}
        })))
}

#[tokio::test]
async fn reading_repository_root_lists_children_without_fetching_a_root_document() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path//@children"))
        .and(query_param("properties", "file,files"))
        .and(query_param("currentPageIndex", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "domain-1", "path": "/default-domain", "title": "Default domain",
                "type": "Domain", "facets": ["Folderish"],
                "properties": {}
            }]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let entries = run_sync(move || {
        let mut cfg = config(base_url);
        cfg.root = "/".to_owned();
        let src = NuxeoSource::connect(cfg).unwrap();
        src.read_dir(&RelPath::root()).unwrap()
    });

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Default domain");
    assert_eq!(entries[0].kind, EntryKind::Dir);
}

#[tokio::test]
async fn route_browser_requests_one_page_and_excludes_non_folder_documents() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path//@children"))
        .and(query_param("properties", "dublincore"))
        .and(query_param("pageSize", "50"))
        .and(query_param("currentPageIndex", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": true,
            "entries": [
                { "uid": "ws-1", "path": "/default-domain/workspaces", "title": "Workspaces",
                  "type": "WorkspaceRoot", "facets": ["Folderish"], "properties": {} },
                { "uid": "file-1", "path": "/default-domain/workspaces/report", "title": "Report",
                  "type": "File", "facets": [], "properties": {} }
            ]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let page = run_sync(move || {
        let mut cfg = config(base_url);
        cfg.root = "/".to_owned();
        let src = NuxeoSource::connect(cfg).unwrap();
        src.browse_directories_page(2).unwrap()
    });

    assert!(page.has_more);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].title, "Workspaces");
    assert_eq!(page.entries[0].path, "/default-domain/workspaces");
}

#[tokio::test]
async fn route_search_escapes_quotes_and_paginates_folderish_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/lang/NXQL/execute"))
        .and(query_param(
            "query",
            "SELECT * FROM Document WHERE ecm:isTrashed = 0 AND ecm:isVersion = 0 AND ecm:isProxy = 0 AND ecm:mixinType = 'Folderish' AND dc:title ILIKE '%Juan''s%' ORDER BY dc:title",
        ))
        .and(query_param("pageSize", "50"))
        .and(query_param("currentPageIndex", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "ws-2", "path": "/default-domain/workspaces/Juan's archive",
                "title": "Juan's archive", "type": "Workspace", "facets": ["Folderish"],
                "properties": {}
            }]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let page = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.search_directories_page("Juan's", 1).unwrap()
    });

    assert!(!page.has_more);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].title, "Juan's archive");
    assert_eq!(
        page.entries[0].path,
        "/default-domain/workspaces/Juan's archive"
    );
}

#[tokio::test]
async fn route_search_uses_nxql_path_prefix_for_absolute_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search/lang/NXQL/execute"))
        .and(query_param(
            "query",
            "SELECT * FROM Document WHERE ecm:isTrashed = 0 AND ecm:isVersion = 0 AND ecm:isProxy = 0 AND ecm:mixinType = 'Folderish' AND ecm:path STARTSWITH '/default-domain/workspaces' ORDER BY dc:title",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": []
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let page = run_sync(move || {
        NuxeoSource::connect(config(base_url))
            .unwrap()
            .search_directories_page("/default-domain/workspaces", 0)
            .unwrap()
    });
    assert!(page.entries.is_empty());
}

#[tokio::test]
async fn read_dir_maps_children_per_the_tree_policy() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [
                {
                    "uid": "1", "path": "/ws/contrato-2024", "title": "Contrato 2024",
                    "type": "File", "lastModified": "2026-09-01T12:00:00.00Z",
                    "facets": ["Versionable"],
                    "properties": {
                        "file:content": {
                            "name": "contrato.docx",
                            "mime-type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                            "length": "587242",
                            "digest": "5d41402abc4b2a76b9719d911017c593",
                            "digestAlgorithm": "MD5"
                        }
                    }
                },
                {
                    "uid": "2", "path": "/ws/acta-junta", "title": "Acta de junta",
                    "type": "File", "lastModified": "2026-09-02T13:30:00.00Z",
                    "facets": [],
                    "properties": {
                        "file:content": { "name": "acta.docx", "length": "2048", "digest": "aa" },
                        "files:files": [
                            { "file": { "name": "acta-firmada.pdf", "mime-type": "application/pdf", "length": "99118", "digest": "bb" } }
                        ]
                    }
                },
                {
                    "uid": "3", "path": "/ws/fotos", "title": "Fotos",
                    "type": "Folder", "lastModified": "2026-08-30T09:00:00.00Z",
                    "facets": ["Folderish"],
                    "properties": {}
                },
                {
                    "uid": "5", "path": "/ws/viejo", "title": "Documento en la papelera",
                    "type": "File", "state": "deleted",
                    "lastModified": "2026-08-01T08:00:00.00Z",
                    "facets": [],
                    "properties": { "file:content": { "name": "viejo.ods", "length": "9", "digest": "ee" } }
                },
                {
                    "uid": "4", "path": "/ws/nota", "title": "Nota",
                    "type": "Note", "lastModified": "2026-08-01T08:00:00.00Z",
                    "facets": [],
                    "properties": { "note:note": "hola" }
                }
            ]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let entries = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root()).unwrap()
    });

    let by_name: std::collections::HashMap<&str, _> =
        entries.iter().map(|e| (e.name.as_str(), e)).collect();
    assert_eq!(
        entries.len(),
        3,
        "blob-less Note and trashed doc are skipped: {entries:?}"
    );

    let contrato = by_name["Contrato 2024.docx"];
    assert_eq!(contrato.kind, EntryKind::File);
    assert_eq!(contrato.size, 587_242);
    assert_eq!(contrato.mtime, Some(1_788_264_000_000)); // 2026-09-01T12:00:00Z

    let acta = by_name["Acta de junta"];
    assert_eq!(
        acta.kind,
        EntryKind::Dir,
        "multi-blob document maps to a directory"
    );

    let fotos = by_name["Fotos"];
    assert_eq!(fotos.kind, EntryKind::Dir);
}

#[tokio::test]
async fn read_dir_on_a_multi_blob_document_lists_its_blobs_deduplicated() {
    let server = MockServer::start().await;
    // Drilling into `acta-junta` resolves its identity from the parent listing first (names are
    // display-only), then fetches the document itself.
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "2", "path": "/ws/acta-junta", "title": "Acta de junta",
                "type": "File", "lastModified": "2026-09-02T13:30:00.00Z",
                "facets": [],
                "properties": {
                    "file:content": { "name": "acta.docx", "length": "2048", "digest": "aa" },
                    "files:files": [
                        { "file": { "name": "acta.pdf", "length": "300", "digest": "cc" } },
                        { "file": { "name": "acta.pdf", "length": "400", "digest": "dd" } }
                    ]
                }
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/acta-junta"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "document",
            "uid": "2",
            "path": "/ws/acta-junta",
            "title": "Acta de junta",
            "type": "File",
            "lastModified": "2026-09-02T13:30:00.00Z",
            "facets": [],
            "properties": {
                "file:content": { "name": "acta.pdf", "length": "2048", "digest": "aa" },
                "files:files": [
                    { "file": { "name": "acta.pdf", "length": "300", "digest": "cc" } },
                    { "file": { "name": "acta.pdf", "length": "400", "digest": "dd" } }
                ]
            }
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let entries = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root().child("Acta de junta"))
            .unwrap()
    });

    let names: Vec<(String, u64)> = entries.iter().map(|e| (e.name.clone(), e.size)).collect();
    assert_eq!(
        names,
        vec![
            ("acta.pdf".to_owned(), 2048),
            ("acta (2).pdf".to_owned(), 300),
            ("acta (3).pdf".to_owned(), 400),
        ]
    );
    assert!(entries.iter().all(|e| e.kind == EntryKind::File));
}

#[tokio::test]
async fn listing_drives_children_pagination_to_completion() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("currentPageIndex", "0"))
        .and(query_param("pageSize", "200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": true,
            "entries": [
                { "uid": "a", "path": "/ws/a", "type": "File", "lastModified": "2026-09-01T12:00:00.00Z",
                  "facets": [], "properties": { "file:content": { "name": "a.txt", "length": "1", "digest": "da" } } },
                { "uid": "b", "path": "/ws/b", "type": "File", "lastModified": "2026-09-01T12:00:00.00Z",
                  "facets": [], "properties": { "file:content": { "name": "b.txt", "length": "2", "digest": "db" } } }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("currentPageIndex", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [
                { "uid": "c", "path": "/ws/c", "type": "File", "lastModified": "2026-09-01T12:00:00.00Z",
                  "facets": [], "properties": { "file:content": { "name": "c.txt", "length": "3", "digest": "dc" } } }
            ]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let entries = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root()).unwrap()
    });
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "b.txt", "c.txt"]);
}

/// The byte-compare differential: a blob served by the (mocked) Nuxeo API must compare equal — via
/// the engine's own `full_equal` — against an identical local file, and unequal against a mutated
/// one. The `Range` header matcher doubles as the assertion that reads are windowed, not whole-file.
#[tokio::test]
async fn open_reads_blobs_with_ranged_gets_and_matches_a_local_file() {
    let content: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let mutated: Vec<u8> = {
        let mut m = content.clone();
        m[2500] = m[2500].wrapping_add(1);
        m
    };

    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    // `open` resolves names through the parent's listing (titles are display-only), so the contract
    // under test is: @children lists the doc → @blob serves it windowed.
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "1", "path": "/ws/contrato-2024",
                "title": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780",
                "type": "File",
                "lastModified": "2026-09-01T12:00:00.00Z",
                "facets": ["Versionable"],
                "properties": {
                    "file:content": {
                        "name": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods",
                        "length": "5000",
                        "digest": "5d41402abc4b2a76b9719d911017c593",
                        "digestAlgorithm": "MD5"
                    }
                }
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/contrato-2024/@blob/file:content"))
        .and(header("Range", "bytes=0-4999"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Range", "bytes 0-4999/5000")
                .set_body_bytes(content.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("same.bin"), &content).unwrap();
    std::fs::write(dir.path().join("differs.bin"), &mutated).unwrap();

    let base_url = server.uri();
    let local_dir = dir.path().to_owned();
    let (same, differs) = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        let local = LocalSource::new(&local_dir);
        let rel =
            RelPath::root().child("2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods");

        let nuxeo = src.open(&rel).unwrap();
        let a = local.open(&RelPath::root().child("same.bin")).unwrap();
        let b = local.open(&RelPath::root().child("differs.bin")).unwrap();
        (
            full_equal(nuxeo.as_ref(), a.as_ref()).unwrap(),
            full_equal(nuxeo.as_ref(), b.as_ref()).unwrap(),
        )
    });
    assert!(same, "identical blob must compare equal via the engine");
    assert!(
        !differs,
        "mutated blob must compare different via the engine"
    );
}

/// A server that ignores `Range` (a plain 200 with the whole body) must not fail the read — that
/// exact response shape broke every >64 KiB copy against the real deployment. The blob downloads
/// once to a temp file and every window is served from disk: exactly ONE HTTP request regardless
/// of how many windows the caller reads.
#[tokio::test]
async fn open_serves_every_window_from_one_fallback_download_when_range_is_ignored() {
    let content: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();

    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "1", "path": "/ws/contrato-2024",
                "title": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780",
                "type": "File",
                "lastModified": "2026-09-01T12:00:00.00Z",
                "facets": ["Versionable"],
                "properties": {
                    "file:content": {
                        "name": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods",
                        "length": "5000",
                        "digest": "5d41402abc4b2a76b9719d911017c593",
                        "digestAlgorithm": "MD5"
                    }
                }
            }]
        })))
        .mount(&server)
        .await;
    // No Range matcher on purpose: the mock answers 200 + whole body to any read.
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/contrato-2024/@blob/file:content"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.clone()))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let windows = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        let rel =
            RelPath::root().child("2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods");
        let reader = src.open(&rel).unwrap();
        let mut first = vec![0u8; 2000];
        let mut middle = vec![0u8; 2000];
        let mut tail = vec![0u8; 2000];
        let n1 = reader.read_at(0, &mut first).unwrap();
        let n2 = reader.read_at(2000, &mut middle).unwrap();
        let n3 = reader.read_at(4000, &mut tail).unwrap();
        (first, n1, middle, n2, tail, n3)
    });
    let (first, n1, middle, n2, tail, n3) = windows;
    assert_eq!((n1, n2, n3), (2000, 2000, 1000));
    assert_eq!(&first[..], &content[..2000]);
    assert_eq!(&middle[..], &content[2000..4000]);
    assert_eq!(&tail[..1000], &content[4000..]);

    let blob_reads = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().contains("@blob"))
        .count();
    assert_eq!(
        blob_reads, 1,
        "windows after the first must come from the local fallback file, not the network"
    );
}

/// A 206 whose body is SHORTER than the requested window is a truncated transfer, not a shorter
/// file: the VFS contract fills interior reads, so the reader must error instead of reporting EOF.
#[tokio::test]
async fn open_rejects_a_truncated_206_interior_window() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "1", "path": "/ws/contrato-2024",
                "title": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780",
                "type": "File",
                "lastModified": "2026-09-01T12:00:00.00Z",
                "facets": ["Versionable"],
                "properties": {
                    "file:content": {
                        "name": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods",
                        "length": "5000",
                        "digest": "5d41402abc4b2a76b9719d911017c593",
                        "digestAlgorithm": "MD5"
                    }
                }
            }]
        })))
        .mount(&server)
        .await;
    // Declared range 0-1999, but only half the body arrives.
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/contrato-2024/@blob/file:content"))
        .and(header("Range", "bytes=0-1999"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Range", "bytes 0-1999/5000")
                .set_body_bytes(vec![7u8; 1000]),
        )
        .mount(&server)
        .await;

    let base_url = server.uri();
    let result = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        let rel =
            RelPath::root().child("2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods");
        let reader = src.open(&rel).unwrap();
        let mut buf = vec![0u8; 2000];
        reader.read_at(0, &mut buf)
    });
    let err = result.err().expect("truncated 206 must fail the read");
    let msg = err.to_string();
    assert!(
        msg.contains("requested 2000 bytes, got 1000"),
        "unexpected error: {msg}"
    );
}

/// The owner's deployment ALSO answers ranged GETs with 200 but only the requested window in the
/// body ("declared N, got 65536"). A 200 whose body is exactly the requested window must serve
/// that window — status codes can't be trusted here, only body lengths.
#[tokio::test]
async fn open_serves_windows_from_200_responses_that_echo_the_requested_range() {
    let content: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();

    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "1", "path": "/ws/contrato-2024",
                "title": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780",
                "type": "File",
                "lastModified": "2026-09-01T12:00:00.00Z",
                "facets": ["Versionable"],
                "properties": {
                    "file:content": {
                        "name": "2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods",
                        "length": "5000",
                        "digest": "5d41402abc4b2a76b9719d911017c593",
                        "digestAlgorithm": "MD5"
                    }
                }
            }]
        })))
        .mount(&server)
        .await;
    for (start, end) in [(0usize, 1999usize), (2000, 3999), (4000, 4999)] {
        Mock::given(method("GET"))
            .and(path("/api/v1/path/ws/contrato-2024/@blob/file:content"))
            .and(header("Range", format!("bytes={start}-{end}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(content[start..=end].to_vec()))
            .mount(&server)
            .await;
    }

    let base_url = server.uri();
    let windows = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        let rel =
            RelPath::root().child("2011010501_presupuesto_CofemaMostoles_DellOptiplex780.ods");
        let reader = src.open(&rel).unwrap();
        let mut first = vec![0u8; 2000];
        let mut middle = vec![0u8; 2000];
        let mut tail = vec![0u8; 2000];
        let n1 = reader.read_at(0, &mut first).unwrap();
        let n2 = reader.read_at(2000, &mut middle).unwrap();
        let n3 = reader.read_at(4000, &mut tail).unwrap();
        (first, n1, middle, n2, tail, n3)
    });
    let (first, n1, middle, n2, tail, n3) = windows;
    assert_eq!((n1, n2, n3), (2000, 2000, 1000));
    assert_eq!(&first[..], &content[..2000]);
    assert_eq!(&middle[..], &content[2000..4000]);
    assert_eq!(&tail[..1000], &content[4000..]);

    let blob_reads = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().contains("@blob"))
        .count();
    assert_eq!(
        blob_reads, 3,
        "each window comes from its own 200 response; none is stored as a fallback"
    );
}

#[tokio::test]
async fn open_resolves_an_attachment_blob_inside_a_document_directory() {
    let scan: Vec<u8> = (0..1000u32).map(|i| (i % 17) as u8).collect();
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "2", "path": "/ws/acta-junta", "title": "Acta de junta",
                "type": "File", "lastModified": "2026-09-02T13:30:00.00Z",
                "facets": [],
                "properties": {
                    "file:content": { "name": "acta.docx", "length": "2048", "digest": "aa" },
                    "files:files": [
                        { "file": { "name": "acta-firmada.pdf", "length": "1000", "digest": "bb" } }
                    ]
                }
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/acta-junta"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "document",
            "uid": "2",
            "path": "/ws/acta-junta",
            "title": "Acta de junta",
            "type": "File",
            "lastModified": "2026-09-02T13:30:00.00Z",
            "facets": [],
            "properties": {
                "file:content": { "name": "acta.docx", "length": "2048", "digest": "aa" },
                "files:files": [
                    { "file": { "name": "acta-firmada.pdf", "length": "1000", "digest": "bb" } }
                ]
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/acta-junta/@blob/files:files/0/file"))
        .and(header("Range", "bytes=0-999"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(scan.clone()))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let (buf, n, len) = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        let rel = RelPath::root()
            .child("Acta de junta")
            .child("acta-firmada.pdf");
        let reader = src.open(&rel).unwrap();
        let len = reader.len();
        let mut buf = vec![0u8; len as usize];
        let n = reader.read_at(0, &mut buf).unwrap();
        (buf, n, len)
    });
    assert_eq!(len, 1000);
    assert_eq!(n as u64, len);
    assert_eq!(buf, scan);
}

#[tokio::test]
async fn quick_fingerprint_surfaces_the_blob_digest() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "1", "path": "/ws/contrato-2024",
                "title": "Contrato 2024",
                "type": "File",
                "lastModified": "2026-09-01T12:00:00.00Z",
                "facets": [],
                "properties": {
                    "file:content": {
                        "name": "contrato.docx",
                        "length": "5000",
                        "digest": "5d41402abc4b2a76b9719d911017c593",
                        "digestAlgorithm": "MD5"
                    }
                }
            }]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let fp = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.quick_fingerprint(&RelPath::root().child("Contrato 2024.docx"))
            .unwrap()
    });
    let expected: Vec<u8> = b"5d41402abc4b2a76b9719d911017c593".to_vec();
    assert_eq!(fp, Some(confold_vfs::Fingerprint(expected)));
}

#[tokio::test]
async fn read_dir_on_a_missing_document_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let result = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root())
    });
    assert!(result.is_err());
}

/// A folder whose Nuxeo path segment diverges from its title (truncation/sanitization/renames) is
/// drillable: `read_dir` must resolve the folder through its cached identity — fetching by the
/// title-derived path would 404. Regression for the owner's "2011 - Presupuestos" report.
#[tokio::test]
async fn read_dir_enters_a_folder_via_identity_when_segment_differs_from_title() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "9", "path": "/ws/2011_-_Presupuestos",
                "title": "2011 - Presupuestos",
                "type": "Folder", "lastModified": "2026-09-03T09:00:00.00Z",
                "facets": ["Folderish"], "properties": {}
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/2011_-_Presupuestos"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "document",
            "uid": "9", "path": "/ws/2011_-_Presupuestos",
            "title": "2011 - Presupuestos",
            "type": "Folder", "lastModified": "2026-09-03T09:00:00.00Z",
            "facets": ["Folderish"], "properties": {}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/2011_-_Presupuestos/@children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "10", "path": "/ws/2011_-_Presupuestos/presupuesto.ods",
                "title": "Presupuesto", "type": "File",
                "lastModified": "2026-09-04T09:00:00.00Z",
                "facets": [],
                "properties": { "file:content": { "name": "presupuesto.ods", "length": "7", "digest": "dd" } }
            }]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let entries = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root().child("2011 - Presupuestos"))
            .unwrap()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Presupuesto.ods");
}

/// Sibling titles that differ only by case map to the same file on case-insensitive destinations
/// (macOS/Windows) — the listing must deduplicate them (` (n)`) instead of pairing a silent
/// overwrite at migrate time.
#[tokio::test]
async fn sibling_titles_differing_only_by_case_are_deduplicated() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [
                { "uid": "1", "path": "/ws/r1", "title": "Presupuesto", "type": "File",
                  "lastModified": "2026-09-01T12:00:00.00Z", "facets": [],
                  "properties": { "file:content": { "name": "a.ods", "length": "1", "digest": "da" } } },
                { "uid": "2", "path": "/ws/r2", "title": "presupuesto", "type": "File",
                  "lastModified": "2026-09-01T12:00:00.00Z", "facets": [],
                  "properties": { "file:content": { "name": "b.ods", "length": "2", "digest": "db" } } }
            ]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let names: Vec<String> = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root()).unwrap()
    })
    .into_iter()
    .map(|e| e.name)
    .collect();
    assert_eq!(names, vec!["Presupuesto.ods", "presupuesto (2).ods"]);
}

/// A title made only of Windows-invalid characters must still produce a creatable file name.
#[tokio::test]
async fn titles_with_reserved_characters_are_sanitized() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{
                "uid": "1", "path": "/ws/q", "title": "Presupuesto: 2025/1 <final?>",
                "type": "File", "lastModified": "2026-09-01T12:00:00.00Z", "facets": [],
                "properties": { "file:content": { "name": "p.ods", "length": "5", "digest": "dq" } } }
            ]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let entries = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root()).unwrap()
    });
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Presupuesto_ 2025_1 _final__.ods");
}

/// Drilling into a folder that exists only on the destination must classify the local side instead
/// of failing: Nuxeo's missing-document error is typed `Io(NotFound)`, which the engine recognizes
/// as an absent side below the compare roots.
#[tokio::test]
async fn read_dir_on_a_missing_nuxeo_child_is_a_typed_not_found() {
    let server = MockServer::start().await;
    ws_doc_mock().mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "file,files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": []
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let err = run_sync(move || {
        let src = NuxeoSource::connect(config(base_url)).unwrap();
        src.read_dir(&RelPath::root().child("solo-en-destino"))
    })
    .unwrap_err();

    match err {
        confold_vfs::SourceError::Io { source, .. } => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected typed Io NotFound, got {other:?}"),
    }
}

/// The route browser lists the CONFIGURED root (the picker overrides root per navigation step) —
/// not the repository root. Regression for the review finding that browsing always showed "/".
#[tokio::test]
async fn route_browser_lists_the_configured_root_not_the_repository_root() {
    let server = MockServer::start().await;
    // Repository root lists a DIFFERENT document set than the configured root /ws.
    Mock::given(method("GET"))
        .and(path("/api/v1/path//@children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{ "uid": "repo", "path": "/default-domain", "title": "Default domain",
                "type": "Domain", "facets": ["Folderish"], "properties": {} }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/path/ws/@children"))
        .and(query_param("properties", "dublincore"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entity-type": "documents",
            "isNextPageAvailable": false,
            "entries": [{ "uid": "ws2", "path": "/ws/workspaces", "title": "Workspaces",
                "type": "WorkspaceRoot", "facets": ["Folderish"], "properties": {} }]
        })))
        .mount(&server)
        .await;

    let base_url = server.uri();
    let page = run_sync(move || {
        NuxeoSource::connect(config(base_url))
            .unwrap()
            .browse_directories_page(0)
            .unwrap()
    });
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].path, "/ws/workspaces");
}
