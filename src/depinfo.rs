use base64::URL_SAFE_NO_PAD;
use blake3::Hash;
use monostate::MustBe;
use semver::Version;
use serde::{Serialize, Serializer};
use std::{
	collections::{BTreeMap, BTreeSet},
	fmt::Display
};

struct HashDef;

impl HashDef {
	fn serialize<S: Serializer>(this: &Hash, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer
	{
		let hash = this.as_bytes();
		let parts = (
			u64::from_be_bytes((&hash[0..8]).try_into().unwrap()),
			u64::from_be_bytes((&hash[8..16]).try_into().unwrap()),
			u64::from_be_bytes((&hash[16..24]).try_into().unwrap()),
			u64::from_be_bytes((&hash[24..32]).try_into().unwrap())
		);
		parts.serialize(serializer)
	}
}

#[derive(Serialize)]
struct DependencyInfoV1 {
	/// The version of this dependency hash. Increase whenever the format of this struct
	/// is changed.
	#[serde(rename = "v")]
	hash_version: MustBe!(1u8),

	/// The version of the markdown output. If there are significant changes made to the
	/// markdown output that require to re-run this tool eventhough none of the inputs
	/// has changed, this version should be increased.
	#[serde(rename = "m")]
	markdown_version: u8,

	/// The blake3 hash of the template file.
	#[serde(rename = "t", with = "HashDef")]
	template_hash: Hash,

	/// The blake3 hash of the input rustdoc.
	#[serde(rename = "r", with = "HashDef")]
	rustdoc_hash: Hash,

	/// The versions of dependencies that are used for link generation. The first entry
	/// of the tuple is the dependency name on crates.io, the second is the version,
	/// and the third is the dependency name as seen in Rust code (or missing if it is
	/// equivalent to the dependency name on crates.io).
	#[serde(rename = "d")]
	dependencies: BTreeSet<(String, Option<Version>, Option<String>)>
}

#[derive(Serialize)]
#[serde(untagged)]
enum DependencyInfoImpl {
	V1(DependencyInfoV1)
}

impl DependencyInfoImpl {
	fn new(markdown_version: u8, template: &str, rustdoc: &str) -> Self {
		Self::V1(DependencyInfoV1 {
			hash_version: Default::default(),
			markdown_version,
			template_hash: blake3::hash(template.as_bytes()),
			rustdoc_hash: blake3::hash(rustdoc.as_bytes()),
			dependencies: BTreeSet::new()
		})
	}

	fn markdown_version(&self) -> u8 {
		match self {
			Self::V1(info) => info.markdown_version
		}
	}

	fn is_template_up2date(&self, template: &str) -> bool {
		match self {
			Self::V1(info) => info.template_hash == blake3::hash(template.as_bytes())
		}
	}

	fn is_rustdoc_up2date(&self, rustdoc: &str) -> bool {
		match self {
			Self::V1(info) => info.rustdoc_hash == blake3::hash(rustdoc.as_bytes())
		}
	}

	fn is_empty(&self) -> bool {
		match self {
			Self::V1(info) => info.dependencies.is_empty()
		}
	}

	fn dependencies(&self) -> BTreeMap<&str, (Option<&Version>, &str)> {
		match self {
			Self::V1(info) => info
				.dependencies
				.iter()
				.map(|(crate_name, version, lib_name)| {
					(
						crate_name.as_str(),
						(version.as_ref(), lib_name.as_deref().unwrap_or(crate_name))
					)
				})
				.collect()
		}
	}

	fn add_dependency(&mut self, crate_name: String, version: Option<Version>, lib_name: String) {
		match self {
			Self::V1(info) => {
				info.dependencies.insert(if lib_name == crate_name {
					(crate_name, version, None)
				} else {
					(crate_name, version, Some(lib_name))
				});
			}
		}
	}
}

pub struct DependencyInfo(DependencyInfoImpl);

impl DependencyInfo {
	pub fn new(markdown_version: u8, template: &str, rustdoc: &str) -> Self {
		Self(DependencyInfoImpl::new(markdown_version, template, rustdoc))
	}

	pub fn encode(&self) -> impl Display {
		base64::encode_config(&serde_cbor::to_vec(&self.0).unwrap(), URL_SAFE_NO_PAD)
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	pub fn add_dependency(
		&mut self,
		crate_name: String,
		version: Option<Version>,
		lib_name: String
	) {
		self.0.add_dependency(crate_name, version, lib_name)
	}
}
