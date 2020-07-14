/*!
 * Basic example that shows a paginated API
 *
 * When you run this program, it will start an HTTP server on an available local
 * port.  See the log entry to see what port it ran on.  Then use curl to use
 * it, like this:
 *
 * ```ignore
 * $ curl localhost:50568/projects
 * ```
 *
 * (Replace 50568 with whatever port your server is listening on.)
 *
 * Try passing different values of the `limit` query parameter.  Try passing the
 * next page token from the response as a query parameter, too.
 */

use chrono::offset::TimeZone;
use chrono::DateTime;
use chrono::Utc;
use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::ExtractedParameter;
use dropshot::HttpError;
use dropshot::HttpResponseOkPage;
use dropshot::HttpServer;
use dropshot::PaginatedResource;
use dropshot::PaginationOrder;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WhichPage;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;
use hyper::Uri;

#[macro_use]
extern crate slog;

/**
 * Object returned by our paginated endpoint
 *
 * Like anything returned by Dropshot, we must implement `JsonSchema` and
 * `Serialize`.  We also implement `Clone` to simplify the example.
 */
#[derive(Clone, Debug, JsonSchema, Serialize)]
struct Project {
    name: String,
    mtime: DateTime<Utc>,
    // lots more fields
}

/**
 * Provided with the first pagination request to specify by what fields the
 * caller wishes to list items and how they are to be sorted.
 * XXX how do we ensure that has a form that's deserializable by
 * serde_querystring?
 */
#[derive(Deserialize, Clone, Debug, ExtractedParameter, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ProjectScanMode {
    /** by name ascending */
    ByNameAscending,
    /** by name descending */
    ByNameDescending,
    /** by mtime descending, then by name ascending */
    ByMtimeDescending,
}

#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
#[serde(rename_all = "lowercase")]
enum ProjectScanPageSelector {
    Name(PaginationOrder, String),
    MtimeName(PaginationOrder, DateTime<Utc>, String),
}

impl From<&ProjectScanPageSelector> for ProjectScanMode {
    fn from(p: &ProjectScanPageSelector) -> ProjectScanMode {
        match p {
            ProjectScanPageSelector::Name(PaginationOrder::Ascending, ..) => {
                ProjectScanMode::ByNameAscending
            }
            ProjectScanPageSelector::Name(PaginationOrder::Descending, ..) => {
                ProjectScanMode::ByNameDescending
            }
            ProjectScanPageSelector::MtimeName(
                PaginationOrder::Descending,
                ..,
            ) => ProjectScanMode::ByMtimeDescending,
            _ => panic!("unsupported mode"), // XXX
        }
    }
}

// XXX shouldn't need to be Deserialize
#[derive(Deserialize)]
struct ProjectScan;

impl PaginatedResource for ProjectScan {
    type ScanMode = ProjectScanMode;
    type PageSelector = ProjectScanPageSelector;
    type Item = Project;

    fn page_selector_for(
        last_item: &Project,
        scan_mode: &ProjectScanMode,
    ) -> ProjectScanPageSelector {
        match scan_mode {
            ProjectScanMode::ByNameAscending => ProjectScanPageSelector::Name(
                PaginationOrder::Ascending,
                last_item.name.clone(),
            ),
            ProjectScanMode::ByNameDescending => ProjectScanPageSelector::Name(
                PaginationOrder::Descending,
                last_item.name.clone(),
            ),
            ProjectScanMode::ByMtimeDescending => {
                ProjectScanPageSelector::MtimeName(
                    PaginationOrder::Descending,
                    last_item.mtime,
                    last_item.name.clone(),
                )
            }
        }
    }
}

/** Default number of returned results */
const DEFAULT_LIMIT: usize = 5;
/** Maximum number of returned results */
const MAX_LIMIT: usize = 100;

/**
 * API endpoint for listing projects
 *
 * This implementation stores all the projects in a BTreeMap, which makes it
 * very easy to fetch a particular range of items based on the key.
 */
#[endpoint {
    method = GET,
    path = "/projects"
}]
async fn example_list_projects(
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<ProjectScan>>,
) -> Result<HttpResponseOkPage<ProjectScan>, HttpError> {
    let pag_params = query.into_inner();
    // XXX even a convenience method here would help
    let mut limit =
        pag_params.limit.map(|l| l.get() as usize).unwrap_or(DEFAULT_LIMIT);
    if limit > MAX_LIMIT {
        limit = MAX_LIMIT;
    }

    // XXX more streamlined way for the library to figure out the list mode
    let data = rqctx_to_data(rqctx);
    let (list_mode, iter) = match &pag_params.page_params {
        WhichPage::FirstPage {
            list_mode: None,
        } => (ProjectScanMode::ByNameAscending, data.iter_by_name_asc()),
        WhichPage::FirstPage {
            list_mode: Some(list_mode @ ProjectScanMode::ByNameAscending),
        } => (list_mode.clone(), data.iter_by_name_asc()),
        WhichPage::FirstPage {
            list_mode: Some(list_mode @ ProjectScanMode::ByNameDescending),
        } => (list_mode.clone(), data.iter_by_name_desc()),
        WhichPage::FirstPage {
            list_mode: Some(list_mode @ ProjectScanMode::ByMtimeDescending),
        } => (list_mode.clone(), data.iter_by_mtime_desc()),
        WhichPage::NextPage {
            page_token: page_params,
        } => {
            let list_mode = ProjectScanMode::from(&page_params.page_start);
            let iter = match &page_params.page_start {
                ProjectScanPageSelector::Name(
                    PaginationOrder::Ascending,
                    name,
                ) => data.iter_by_name_asc_from(name),
                ProjectScanPageSelector::Name(
                    PaginationOrder::Descending,
                    name,
                ) => data.iter_by_name_desc_from(name),
                ProjectScanPageSelector::MtimeName(
                    PaginationOrder::Ascending,
                    mtime,
                    name,
                ) => data.iter_by_mtime_asc_from(mtime, name),
                ProjectScanPageSelector::MtimeName(
                    PaginationOrder::Descending,
                    mtime,
                    name,
                ) => data.iter_by_mtime_desc_from(mtime, name),
            };
            (list_mode, iter)
        }
    };

    let projects = iter.take(limit).map(|p| (*p).clone()).collect();

    Ok(HttpResponseOkPage(list_mode, projects))
}

fn rqctx_to_data(rqctx: Arc<RequestContext>) -> Arc<ProjectCollection> {
    let c = Arc::clone(&rqctx.server.private);
    c.downcast::<ProjectCollection>().unwrap()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    /*
     * Create 1000 projects up front.
     */
    let mut data = ProjectCollection {
        by_name: BTreeMap::new(),
        by_mtime: BTreeMap::new(),
    };
    let mut timestamp = DateTime::parse_from_rfc3339("2020-07-13T17:35:00Z")
        .unwrap()
        .timestamp_millis();
    for n in 1..1000 {
        let name = format!("project{:03}", n);
        let project = Arc::new(Project {
            name: name.clone(),
            mtime: Utc.timestamp_millis(timestamp),
        });
        /*
         * To make this dataset at least somewhat interesting in terms of
         * exercising different pagination parameters, we'll make the mtimes
         * decrease with the names, and we'll have some objects with the same
         * mtime.
         */
        if n % 10 != 0 {
            timestamp = timestamp - 1;
        }
        data.by_name.insert(name.clone(), Arc::clone(&project));
        data.by_mtime.insert((project.mtime, name), project);
    }

    /*
     * Run the Dropshot server.
     */
    let ctx = Arc::new(data);
    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
    };
    let config_logging = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Debug,
    };
    let log = config_logging
        .to_logger("example-pagination-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_list_projects).unwrap();
    let mut server = HttpServer::new(&config_dropshot, api, ctx, &log)
        .map_err(|error| format!("failed to create server: {}", error))?;
    let server_task = server.run();

    /*
     * Dump out some useful starting points.
     */
    #[derive(Serialize)]
    struct ToPrint {
        list_mode: ProjectScanMode,
    };
    let all_modes = vec![
        ProjectScanMode::ByNameAscending,
        ProjectScanMode::ByNameDescending,
        ProjectScanMode::ByMtimeDescending,
    ];
    let local_addr = server.local_addr();
    for mode in all_modes {
        let to_print = ToPrint { list_mode: mode };
        let query_string = serde_urlencoded::to_string(to_print).unwrap();
        let uri = Uri::builder()
            .scheme("http")
            .authority(local_addr.to_string().as_str())
            .path_and_query(format!("/projects?{}", query_string).as_str())
            .build()
            .unwrap();
        info!(log, "example: {}", uri);
    }

    server.wait_for_shutdown(server_task).await
}

struct ProjectCollection {
    by_name: BTreeMap<String, Arc<Project>>,
    by_mtime: BTreeMap<(DateTime<Utc>, String), Arc<Project>>,
}

type ProjectIter<'a> = Box<dyn Iterator<Item = Arc<Project>> + 'a>;

impl ProjectCollection {
    fn iter_by_name_asc(&self) -> ProjectIter {
        self.make_iter(self.by_name.iter())
    }
    fn iter_by_name_desc(&self) -> ProjectIter {
        self.make_iter(self.by_name.iter().rev())
    }
    fn iter_by_name_asc_from(&self, last_seen: &String) -> ProjectIter {
        let iter = self
            .by_name
            .range((Bound::Excluded(last_seen.clone()), Bound::Unbounded));
        self.make_iter(iter)
    }
    fn iter_by_name_desc_from(&self, last_seen: &String) -> ProjectIter {
        let iter = self
            .by_name
            .range((Bound::Unbounded, Bound::Excluded(last_seen.clone())))
            .rev();
        self.make_iter(iter)
    }

    fn make_iter<'a, K, I>(&'a self, iter: I) -> ProjectIter<'a>
    where
        I: Iterator<Item = (K, &'a Arc<Project>)> + 'a,
    {
        Box::new(iter.map(|(_, project)| Arc::clone(project)))
    }

    fn iter_by_mtime_asc(&self) -> ProjectIter {
        self.make_iter(self.by_mtime.iter())
    }
    fn iter_by_mtime_desc(&self) -> ProjectIter {
        self.make_iter(self.by_mtime.iter().rev())
    }

    fn iter_by_mtime_asc_from(
        &self,
        last_mtime: &DateTime<Utc>,
        last_name: &String,
    ) -> ProjectIter {
        let last_seen = &(*last_mtime, last_name.clone());
        let iter =
            self.by_mtime.range((Bound::Excluded(last_seen), Bound::Unbounded));
        self.make_iter(iter)
    }

    fn iter_by_mtime_desc_from(
        &self,
        last_mtime: &DateTime<Utc>,
        last_name: &String,
    ) -> ProjectIter {
        let last_seen = &(*last_mtime, last_name.clone());
        let iter = self
            .by_mtime
            .range((Bound::Unbounded, Bound::Excluded(last_seen)))
            .rev();
        self.make_iter(iter)
    }
}
