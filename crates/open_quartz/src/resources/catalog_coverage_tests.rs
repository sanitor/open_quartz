use super::*;

#[test]
fn resources_cover_kind_source_and_catalog_emptiness() {
    let mut catalog = ResourceCatalog::default();
    assert!(catalog.is_empty());
    let resource = Resource::new(
        ResourceId::new("image"),
        ResourceKind::Image,
        ResourceSource::Path("image.png".to_owned()),
    );
    assert_eq!(resource.kind(), ResourceKind::Image);
    assert_eq!(
        resource.source(),
        &ResourceSource::Path("image.png".to_owned())
    );
    catalog.add(resource).unwrap();
    assert!(!catalog.is_empty());
}

#[test]
fn resource_catalog_covers_ids_length_and_duplicate_rejection() {
    let mut resources = ResourceCatalog::default();
    let resource = Resource::new(
        ResourceId::new("resource"),
        ResourceKind::Video,
        ResourceSource::Url("https://example.invalid/video".to_owned()),
    );
    assert_eq!(resource.id().as_str(), "resource");
    resources.add(resource.clone()).unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(
        resources.add(resource).unwrap_err().code,
        SdkErrorCode::InvalidResource
    );
}
