use std::sync::Arc;

use crate::Type;

/// A tuple element's label: the `NamedTupleMember` (or parameter) name tsc
/// keeps on the tuple target as `labeledElementDeclarations`.
pub type TupleLabel = Option<Arc<str>>;

/// A fixed-length tuple: its element types, and what tsc's tuple target keeps
/// beside them — the element flags (as the `minLength` they give) and labels.
/// A tuple surge builds from element types alone has no shape; its
/// `minLength` is read off the types ([`crate::tuple_min_length`]), where an
/// optional element carries its `undefined`. Like tsc's tuple targets, tuples
/// with different flags or labels are different types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TupleType {
    elements: Vec<Type>,
    shape: Option<Arc<TupleShape>>,
}

#[derive(Debug, PartialEq, Eq)]
struct TupleShape {
    min_length: usize,
    labels: Option<Box<[TupleLabel]>>,
}

impl TupleType {
    pub fn new(elements: Vec<Type>) -> Self {
        Self { elements, shape: None }
    }

    /// A tuple whose first `min_length` elements are required, labelled when
    /// `labels` names every position.
    pub fn with_shape(elements: Vec<Type>, min_length: usize, labels: Option<Vec<TupleLabel>>) -> Self {
        let shape = Some(tuple_shape(&elements, min_length, labels));
        Self { elements, shape }
    }

    /// A tuple read off its element types, labelled when `labels` names any
    /// position.
    pub fn labeled(elements: Vec<Type>, labels: Option<Vec<TupleLabel>>) -> Self {
        if !labels.as_ref().is_some_and(|labels| labels.len() == elements.len() && labels.iter().any(Option::is_some)) {
            return Self::new(elements);
        }
        let min_length = crate::tuple_min_length(&elements);
        Self::with_shape(elements, min_length, labels)
    }

    pub fn elements(&self) -> &[Type] {
        &self.elements
    }

    pub fn as_slice(&self) -> &[Type] {
        &self.elements
    }

    pub fn into_elements(self) -> Vec<Type> {
        self.elements
    }

    pub fn min_length(&self) -> usize {
        match &self.shape {
            Some(shape) => shape.min_length,
            None => crate::tuple_min_length(&self.elements),
        }
    }

    pub fn is_optional(&self, index: usize) -> bool {
        index >= self.min_length()
    }

    /// Whether the element at `index` is flagged optional — known only for a
    /// tuple with a shape.
    pub fn is_flagged_optional(&self, index: usize) -> bool {
        self.shape.as_ref().is_some_and(|shape| index >= shape.min_length)
    }

    pub fn labels(&self) -> Option<&[TupleLabel]> {
        self.shape.as_ref()?.labels.as_deref()
    }

    pub fn label(&self, index: usize) -> Option<&str> {
        self.labels()?.get(index)?.as_deref()
    }

    /// The same tuple over new element types, keeping each position's flag
    /// and label — an instantiation of the same tuple target.
    pub fn map_elements(&self, f: impl FnMut(&Type) -> Type) -> Self {
        let elements = self.elements.iter().map(f).collect();
        self.with_elements(elements)
    }

    /// `sliceTupleType`: the elements from `start` to `end`, keeping their
    /// flags and labels.
    pub fn slice(&self, start: usize, end: usize) -> Self {
        let elements = self.elements[start..end].to_vec();
        match &self.shape {
            Some(shape) => {
                let min_length = shape.min_length.clamp(start, end) - start;
                let labels = shape.labels.as_ref().map(|labels| labels[start..end].to_vec());
                Self::with_shape(elements, min_length, labels)
            }
            None => Self::new(elements),
        }
    }

    /// New element types at the same positions; a different arity drops the
    /// shape.
    fn with_elements(&self, elements: Vec<Type>) -> Self {
        match &self.shape {
            Some(shape) if elements.len() == self.elements.len() => Self { elements, shape: Some(shape.clone()) },
            _ => Self::new(elements),
        }
    }
}

fn tuple_shape(elements: &[Type], min_length: usize, labels: Option<Vec<TupleLabel>>) -> Arc<TupleShape> {
    let min_length = min_length.min(elements.len());
    let labels = labels
        .filter(|labels| labels.len() == elements.len() && labels.iter().any(Option::is_some))
        .map(Vec::into_boxed_slice);
    Arc::new(TupleShape { min_length, labels })
}

/// An open tuple's [`crate::OpenTupleType::labels`]: kept only when some
/// position is labelled.
pub fn open_tuple_labels(labels: Vec<TupleLabel>) -> Option<Arc<[TupleLabel]>> {
    labels.iter().any(Option::is_some).then(|| Arc::from(labels))
}

impl std::ops::Deref for TupleType {
    type Target = [Type];

    fn deref(&self) -> &[Type] {
        &self.elements
    }
}

impl From<Vec<Type>> for TupleType {
    fn from(elements: Vec<Type>) -> Self {
        Self::new(elements)
    }
}

/// Collecting element types builds an unlabelled tuple; an instantiation that
/// keeps the target's flags and labels goes through [`TupleType::map_elements`].
impl FromIterator<Type> for TupleType {
    fn from_iter<I: IntoIterator<Item = Type>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a TupleType {
    type Item = &'a Type;
    type IntoIter = std::slice::Iter<'a, Type>;

    fn into_iter(self) -> Self::IntoIter {
        self.elements.iter()
    }
}

impl IntoIterator for TupleType {
    type Item = Type;
    type IntoIter = std::vec::IntoIter<Type>;

    fn into_iter(self) -> Self::IntoIter {
        self.elements.into_iter()
    }
}

impl Type {
    pub fn tuple(elements: Vec<Type>) -> Type {
        Type::Tuple(TupleType::new(elements))
    }
}
