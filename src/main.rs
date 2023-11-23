use clap::Parser;
use git2::{Commit, ObjectType, Oid, Repository, Signature};
use regex::{Match, Regex};
use std::{collections::HashMap, fs::File, io::Write, path::PathBuf};
use thiserror::Error;

const DEBUG: bool = true;

#[derive(Error, Debug)]
pub enum VersionError {
    #[error("Error: {0}")]
    Generic(String),
    #[error("git error")]
    Git(#[from] git2::Error),
    #[error("io error")]
    Io(#[from] std::io::Error),
}

impl From<&str> for VersionError {
    fn from(s: &str) -> Self {
        VersionError::Generic(s.to_string())
    }
}

/// What the branch-name tells us about the version
#[derive(Debug)]
enum BranchVersion {
    // master, main, release
    Master,
    // release: 1.2.3
    Release(SemanticVersion),
    // feature/myfeature
    Feature(String),
    // fix/myfix
    Fix(String),
    // other
    Other(String),
}

/// takes a repository and returns the branch name
/// if the repository is a git repository, otherwise returns Error
fn branch_version(repo: &Repository) -> Result<BranchVersion, VersionError> {
    let head = repo.head()?;
    let branch = head.shorthand().unwrap().to_lowercase();

    match version_from_string(&branch, None) {
        Some(v) => Ok(BranchVersion::Release(v)),
        None => {
            if branch == "master" || branch == "main" || branch == "release" {
                Ok(BranchVersion::Master)
            } else if branch.starts_with("feature/") {
                Ok(BranchVersion::Feature(branch.clone().split_off(8)))
            } else if branch.starts_with("fix/") {
                Ok(BranchVersion::Fix(branch.clone().split_off(4)))
            } else {
                Ok(BranchVersion::Other(branch.to_string()))
            }
        }
    }
}

/// a major.minor.patch version
#[derive(Debug)]
struct SemanticVersion {
    major: usize,
    minor: usize,
    patch: usize,
    ident: Option<String>,
    commit: Option<String>,
}

impl SemanticVersion {
    fn new(
        major: usize,
        minor: usize,
        patch: usize,
        ident: Option<String>,
        commit: Option<String>,
    ) -> Self {
        Self {
            major,
            minor,
            patch,
            ident,
            commit,
        }
    }

    // fn from_triple(major: usize, minor: usize, patch: usize) -> Self {
    //     SemanticVersion::new(major, minor, patch, None, None)
    // }

    fn version_string(&self) -> String {
        match self.ident {
            Some(ref v) => format!("{}.{}.{}-{}", self.major, self.minor, self.patch, v),
            None => format!("{}.{}.{}", self.major, self.minor, self.patch),
        }
    }

}

fn version_from_string(raw_name: &str, commit: Option<&Commit>) -> Option<SemanticVersion> {
    let re = Regex::new(r"([a-z])?(\d+)([\.\-](\d+))?([\.\-](\d+))?").unwrap();

    let name = raw_name.to_lowercase();
    let commit = commit.and_then(|c| c.as_object().short_id().ok())
        .and_then(|b| b.as_str().and_then(|s| Some(String::from(s))));

    match re.captures(&name) {
        Some(caps) => {
            let major = to_number(caps.get(2));
            let minor = to_number(caps.get(4));
            let patch = to_number(caps.get(6));

            println!("caps: {:?}", caps);
            println!("semver: {} {} {}", major, minor, patch);
            // let minor = caps.get(2).unwrap().as_str();
            Some(SemanticVersion::new(major, minor, patch, None, commit))
        }
        None => None,
    }
}

fn to_number(s: Option<Match>) -> usize {
    match s {
        Some(s) => s.as_str().parse::<usize>().unwrap_or(0),
        None => 0,
    }
}

/// a version for a commit that is a few commits (patches)
/// away from a release version
#[derive(Debug)]
struct PatchVersion {
    release: Option<SemanticVersion>,
    patch_count: usize,
    _patch_oid: Option<Oid>,
    patch_short: Option<String>,
    ident: Option<String>,
}

impl PatchVersion {
    fn new(
        release: SemanticVersion,
        distance: usize,
        ident: Option<String>,
        oid: Option<Oid>,
        short: Option<String>,
    ) -> Self {
        Self {
            release: Some(release),
            patch_count: distance,
            _patch_oid: oid,
            patch_short: short,
            ident: ident,
        }
    }

    fn semver(&self) -> SemanticVersion {
        match self.release {
            Some(ref rv) => SemanticVersion::new(
                rv.major,
                rv.minor,
                rv.patch + self.patch_count,
                self.ident.clone(),
                self.patch_short.clone(),
            ),
            None => SemanticVersion::new(
                0,
                0,
                self.patch_count,
                self.ident.clone(),
                self.patch_short.clone(),
            ),
        }
    }
}

fn head_version(repo: &Repository) -> Result<PatchVersion, VersionError> {
    // map with all tags in the repository
    let tagmap: HashMap<Oid, FullTag> = repo
        .tag_names(None)?
        .iter()
        .filter_map(|n| n)
        .filter_map(|n| {
            if let Ok(t) = resolve_tag(repo, n) {
                Some((t.target, t))
            } else {
                None
            }
        })
        .collect();

    let head = repo.head()?;
    let head_oid = head.target().ok_or(VersionError::from("no target"))?;
    let head_short = repo
        .find_object(head_oid, None)?
        .short_id()?
        .as_str()
        .unwrap_or("0000000")
        .to_string();

    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    revwalk.set_sorting(git2::Sort::TIME)?;
    revwalk.simplify_first_parent()?;

    let mut count = 0;

    for roid in revwalk {
        let oid = roid?;

        // find the commit
        let commit = repo.find_commit(oid)?;

        // check if the commit is a release commit
        if let Some(cm) = commit.message() {
            if cm.to_lowercase().starts_with("release:") {
                if let Some(rv) = version_from_string(cm, Some(&commit)) {
                    println!(
                        "commit-rv: {:?} {:?} {}",
                        cm,
                        rv,
                        commit.as_object().short_id()?.as_str().unwrap_or("?")
                    );
                    return Ok(PatchVersion::new(
                        rv,
                        count,
                        None,
                        Some(head_oid),
                        Some(head_short),
                    ));
                }
            }
        }

        // check if there is a tag for that commit
        if let Some(tag) = tagmap.get(&oid) {
            if let Some(rv) = version_from_string(&tag.name, Some(&commit)) {
                println!("tag-rv: {:?} {:?}", tag.name, rv);
                return Ok(PatchVersion::new(
                    rv,
                    count,
                    None,
                    Some(head_oid),
                    Some(head_short),
                ));
            }
        }

        println!("{} {}", oid, commit.summary().unwrap());

        count += 1;
        if count > 4096 {
            return Err(VersionError::from("too many commits"));
        }
    }

    return Ok(PatchVersion::new(
        SemanticVersion::new(0, 0, 0, None, None),
        count,
        None,
        Some(head_oid),
        Some(head_short),
    ));
}

#[derive(Debug)]
struct FullTag {
    name: String,
    target: Oid,
}

fn resolve_tag(repo: &Repository, name: &str) -> Result<FullTag, git2::Error> {
    let tref = repo.resolve_reference_from_short_name(name)?;
    let tag = tref.peel_to_tag()?;
    let target = tag.target_id();

    Ok(FullTag {
        name: name.to_string(),
        target: target,
    })
}

/// Find version for current git commit.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the Git Repository
    #[arg(short, long, default_value_t = String::from("."))]
    git: String,

    #[arg(short, long)]
    release: bool,
}

/// Make a release
fn main_release(repo: &Repository) -> Result<SemanticVersion, VersionError> {
    let current_version = main_version(repo)?;
    if current_version.patch == 0 {
        eprintln!(
            "patch version is not zero, we are already on a release commit: {}",
            current_version.version_string()
        );
        return Err(VersionError::Generic(
            "patch version is not zero".to_string(),
        ));
    }

    let next_version = SemanticVersion::new(
        current_version.major,
        current_version.minor + 1,
        0,
        None,
        None,
    );

    let filename = format!("changes.{}", next_version.version_string());

    let workdir = repo.workdir().ok_or(git2::Error::from_str("no workdir"))?;
    let changes = workdir.join(&filename);

    let mut cfile = File::create(&changes)?;
    writeln!(
        cfile,
        "Changes for version {}",
        next_version.version_string()
    )?;
    cfile.flush()?;

    let obj = repo.head()?.resolve()?.peel(ObjectType::Commit)?;

    let mut index = repo.index()?;
    index.add_path(&PathBuf::from(&filename))?;

    let oid = index.write_tree()?;
    let signature = Signature::now("Peter Panoo", "peter@panoo.com")?;
    let parent_commit = obj
        .into_commit()
        .map_err(|_| git2::Error::from_str("not a commit"))?;
    let tree = repo.find_tree(oid)?;

    let message = format!("release: {}", next_version.version_string());

    let nexthead = repo.commit(
        Some("HEAD"), //  point HEAD to our new commit
        &signature,   // author
        &signature,   // committer
        &message,     // commit message
        &tree,        // tree
        &[&parent_commit],
    )?;

    let nextobj = repo.find_object(nexthead, None)?;

    let ident = match next_version.ident {
        Some(ref v) => format!("-{}", v),
        None => String::new(),
    };

    let panoo_version = format!("P{}-{}{}", next_version.major, next_version.minor, ident);
    let panoo_message = format!("Release {}", &panoo_version);
    repo.tag(&panoo_version, &nextobj, &signature, &panoo_message, true)?;

    Ok(next_version)
}

fn nmerge(branch: usize, head: usize) -> Result<usize, VersionError> {
    if branch == 0 || head == 0 {
        Ok(head + branch)
    } else if branch == head {
        Ok(branch)
    } else {
        Err(VersionError::from("major version mismatch"))
    }
}

// fn smerge(branch: &Option<String>, head: &Option<String>) -> Option<String> {
//     if branch.is_none() && head.is_none() {
//         None
//     } else if head.is_none() {
//         branch.clone()
//     } else if branch.is_none() {
//         head.clone()
//     } else {
//         branch.clone()
//     }
// }

/// Return a version for the current git commit.
fn main_version(repo: &Repository) -> Result<SemanticVersion, VersionError> {
    // check the branch itself for version information
    let branch = branch_version(repo)?;
    if DEBUG {
        println!("Branch: {:?}", branch);
    }

    let head = head_version(repo)?;
    let headv = head.semver();

    if DEBUG {
        println!("Head: {:?}", head);
    }

    let bv = match branch {
        BranchVersion::Master => head.semver(),
        BranchVersion::Release(branchv) => {
            let major = nmerge(branchv.major, headv.major)?;
            let minor = nmerge(branchv.minor, headv.minor)?;
            let patch = headv.patch;

            SemanticVersion::new(major, minor, patch, None, headv.commit)
        }
        BranchVersion::Feature(f) => {
            SemanticVersion::new(headv.major, headv.minor, headv.patch, Some(f), headv.commit)
        }
        BranchVersion::Fix(f) => {
            SemanticVersion::new(headv.major, headv.minor, headv.patch, Some(f), headv.commit)
        }
        BranchVersion::Other(_f) => SemanticVersion::new(
            headv.major,
            headv.minor,
            headv.patch,
            Some(String::from("other")),
            headv.commit,
        ),
    };

    Ok(bv)
}

fn main_result(args: Args) -> Result<(), VersionError> {
    let repo = Repository::open(args.git)?;

    if args.release {
        let _ = main_release(&repo).unwrap();
    } else {
        let v = main_version(&repo)?;
        println!("{}", v.version_string());
    }

    // let stats = repo.statuses(None).unwrap();

    // stats.iter().for_each(|s| {
    //     println!("state: {:?}", s.status());
    // });

    // let _ = main_version(&repo).unwrap();

    //     print_last_100_commits(&repo).unwrap();

    // let branch_version = get_branch_version(&repo);
    // let tagged_version = get_tagged_version(&repo);

    // match build_new_version(&repo) {
    //     Ok(version) => println!("version: {}", version),
    //     Err(e) => println!("error: {}", e),
    // }

    Ok(())
}

fn main() {
    let args = Args::parse();

    if let Err(e) = main_result(args) {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}
