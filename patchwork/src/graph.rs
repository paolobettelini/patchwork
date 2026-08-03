use crate::error::{PatchworkError, Result};
use crate::model::ModInfo;
use std::collections::{HashMap, HashSet, VecDeque};

pub struct ResolvedGraph {
    pub mods: Vec<(String, ModInfo)>,
    pub provider_map: HashMap<String, String>,
    pub owned_objects: HashSet<String>,
}

pub fn resolve(mut mods_map: HashMap<String, ModInfo>) -> Result<ResolvedGraph> {
    let provider_map = build_provider_map(&mods_map)?;
    let owned_objects = validate_ownership(&mods_map, &provider_map)?;
    let order = topological_order(&mods_map, &provider_map)?;

    let mut mods = Vec::with_capacity(order.len());
    for name in order {
        let info = mods_map
            .remove(&name)
            .expect("mod should exist in map at this point");
        mods.push((name, info));
    }

    Ok(ResolvedGraph {
        mods,
        provider_map,
        owned_objects,
    })
}

fn build_provider_map(mods_map: &HashMap<String, ModInfo>) -> Result<HashMap<String, String>> {
    let mut provider_map: HashMap<String, String> = HashMap::new();

    for (name, info) in mods_map {
        if let Some(prov) = &info.provides {
            if provider_map.contains_key(prov) {
                return Err(PatchworkError::DuplicateProvider {
                    api: prov.clone(),
                    first_provider: provider_map.get(prov).unwrap().clone(),
                    second_provider: name.clone(),
                });
            }
            provider_map.insert(prov.clone(), name.clone());
        }
    }

    for (api, provider) in &provider_map {
        match mods_map.get(api) {
            Some(info) if info.api => {}
            Some(_) => {
                return Err(PatchworkError::InvalidApiProviderTarget {
                    api: api.clone(),
                    provider: provider.clone(),
                });
            }
            None => {
                return Err(PatchworkError::MissingDependency {
                    dependent_mod: provider.clone(),
                    dependency: api.clone(),
                });
            }
        }
    }

    for (name, info) in mods_map {
        if info.api && !provider_map.contains_key(name) {
            return Err(PatchworkError::MissingApiProvider { api: name.clone() });
        }
    }

    Ok(provider_map)
}

fn validate_ownership(
    mods_map: &HashMap<String, ModInfo>,
    provider_map: &HashMap<String, String>,
) -> Result<HashSet<String>> {
    let mut ownership_of = HashMap::new();
    let mut run_users: HashMap<String, Vec<String>> = HashMap::new();

    for (name, info) in mods_map {
        for obj in &info.dependencies.ownership {
            let resolved = resolve_dependency_name(name, obj, mods_map, provider_map)?;

            if let Some(other_mod) = ownership_of.get(&resolved) {
                return Err(PatchworkError::OwnershipConflict {
                    message: format!(
                        "ownership conflict: both '{}' and '{}' claim ownership of '{}'",
                        other_mod, name, obj
                    ),
                });
            }
            ownership_of.insert(resolved, name.clone());
        }

        for obj in &info.dependencies.run {
            let resolved = resolve_dependency_name(name, obj, mods_map, provider_map)?;
            run_users.entry(resolved).or_default().push(name.clone());
        }
    }

    for (obj, owner) in &ownership_of {
        if let Some(users) = run_users.get(obj) {
            let other_users = users
                .iter()
                .filter(|user| *user != owner)
                .cloned()
                .collect::<Vec<_>>();
            if !other_users.is_empty() {
                return Err(PatchworkError::OwnershipConflict {
                    message: format!(
                        "ownership conflict: '{}' takes ownership of '{}', but {} also request it in run",
                        owner,
                        obj,
                        other_users.join(", ")
                    ),
                });
            }
        }
    }

    Ok(ownership_of.keys().cloned().collect())
}

fn topological_order(
    mods_map: &HashMap<String, ModInfo>,
    provider_map: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    for name in mods_map.keys() {
        graph.insert(name.clone(), Vec::new());
        in_degree.insert(name.clone(), 0);
    }

    let mut added_edges = HashSet::new();

    for (name, info) in mods_map {
        let all_deps = info
            .dependencies
            .init
            .iter()
            .chain(info.dependencies.run.iter())
            .chain(info.dependencies.ownership.iter())
            .collect::<Vec<_>>();

        for dep in all_deps {
            let dep_mod = resolve_dependency_name(name, dep, mods_map, provider_map)?;

            if dep_mod == *name {
                return Err(PatchworkError::SelfDependency {
                    mod_name: name.clone(),
                });
            }

            let edge = (dep_mod.clone(), name.clone());
            if !added_edges.insert(edge) {
                continue;
            }

            graph
                .get_mut(&dep_mod)
                .expect("graph entry exists")
                .push(name.clone());

            *in_degree
                .get_mut(name)
                .expect("in_degree entry exists for dependent") += 1;
        }
    }

    let mut q = VecDeque::new();
    for (name, &deg) in &in_degree {
        if deg == 0 {
            q.push_back(name.clone());
        }
    }

    let mut order = Vec::new();
    while let Some(node) = q.pop_front() {
        order.push(node.clone());
        for neighbor in graph.get(&node).unwrap() {
            let deg = in_degree.get_mut(neighbor).unwrap();
            *deg -= 1;
            if *deg == 0 {
                q.push_back(neighbor.clone());
            }
        }
    }

    if order.len() != mods_map.len() {
        let mut unresolved_mods = in_degree
            .into_iter()
            .filter_map(|(name, degree)| (degree > 0).then_some(name))
            .collect::<Vec<_>>();
        unresolved_mods.sort();
        return Err(PatchworkError::ModDependencyCycle { unresolved_mods });
    }

    Ok(order)
}

fn resolve_dependency_name(
    dependent_mod: &str,
    dep: &str,
    mods_map: &HashMap<String, ModInfo>,
    provider_map: &HashMap<String, String>,
) -> Result<String> {
    if mods_map.get(dep).is_some_and(|info| info.api) {
        if let Some(provider) = provider_map.get(dep) {
            Ok(provider.clone())
        } else {
            Err(PatchworkError::MissingApiProvider {
                api: dep.to_string(),
            })
        }
    } else if mods_map.get(dep).is_some_and(|info| info.support) {
        Err(PatchworkError::NonLifecycleDependency {
            dependent_mod: dependent_mod.to_string(),
            dependency: dep.to_string(),
        })
    } else if mods_map.contains_key(dep) {
        Ok(dep.to_string())
    } else if let Some(provider) = provider_map.get(dep) {
        Ok(provider.clone())
    } else {
        Err(PatchworkError::MissingDependency {
            dependent_mod: dependent_mod.to_string(),
            dependency: dep.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CodegenDeclaration, Dependencies};

    fn mod_info(entry: Option<&str>, api: bool, support: bool, provides: Option<&str>) -> ModInfo {
        ModInfo {
            title: None,
            entry: entry.map(str::to_owned),
            dependencies: Dependencies::default(),
            provides: provides.map(str::to_owned),
            support,
            api,
            codegen: Vec::<CodegenDeclaration>::new(),
        }
    }

    #[test]
    fn api_mod_requires_exactly_one_selected_provider() {
        let mut mods = HashMap::new();
        mods.insert(
            "inventory-api".to_owned(),
            mod_info(None, true, false, None),
        );

        assert!(matches!(
            resolve(mods),
            Err(PatchworkError::MissingApiProvider { api }) if api == "inventory-api"
        ));
    }

    #[test]
    fn api_dependency_resolves_to_provider_while_api_remains_selected() {
        let mut consumer = mod_info(Some("Consumer"), false, false, None);
        consumer.dependencies.run.push("inventory-api".to_owned());

        let mut mods = HashMap::new();
        mods.insert(
            "inventory-api".to_owned(),
            mod_info(None, true, false, None),
        );
        mods.insert(
            "inventory-default".to_owned(),
            mod_info(
                Some("InventoryDefault"),
                false,
                false,
                Some("inventory-api"),
            ),
        );
        mods.insert("consumer".to_owned(), consumer);

        let resolved = resolve(mods).unwrap();
        assert_eq!(
            resolved
                .provider_map
                .get("inventory-api")
                .map(String::as_str),
            Some("inventory-default")
        );
        assert!(
            resolved
                .mods
                .iter()
                .any(|(name, info)| name == "inventory-api" && info.api)
        );
    }
}
