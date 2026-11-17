//! Port of xpdf `Catalog.h` / `Catalog.cc`: lazy page-tree walking with
//! attribute inheritance (`PageTreeNode`, `loadPage2`), page counting,
//! `findPage` and named destinations (`/Dests` dictionary and the
//! `/Names /Dests` name tree).
//!
//! Not ported (no effect on what pdfTeX reads): AcroForm loading,
//! embedded file lists, page labels, base URI, JavaScript scanning.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::{error, ErrorCategory};
use crate::link::LinkDest;
use crate::object::{Object, Ref};
use crate::page::{Page, PageAttrs};
use crate::xref::XRef;

/// Mirrors `class PageTreeNode` (stored in an arena; `parent`/`kids` are
/// indices).
struct PageTreeNode {
    ref_: Ref,
    count: i32,
    parent: Option<usize>,
    kids: Option<Vec<usize>>,
    attrs: Option<PageAttrs>,
}

struct PageState {
    nodes: Vec<PageTreeNode>,
    pages: Vec<Option<Rc<Page>>>,
    page_refs: Vec<Ref>,
}

/// Mirrors `class Catalog`.
pub struct Catalog {
    xref: Rc<XRef>,
    ok: bool,
    num_pages: i32,
    state: RefCell<PageState>,
    dests: Object,
    name_tree: Object,
    metadata: Object,
    struct_tree_root: Object,
    outline: Object,
    acro_form: Object,
    needs_rendering: bool,
    oc_properties: Object,
    viewer_prefs: Object,
}

impl Catalog {
    /// `Catalog::Catalog(PDFDoc *docA)`.
    pub fn new(xref: Rc<XRef>) -> Catalog {
        let mut c = Catalog {
            xref: xref.clone(),
            ok: true,
            num_pages: 0,
            state: RefCell::new(PageState { nodes: Vec::new(), pages: Vec::new(), page_refs: Vec::new() }),
            dests: Object::Null,
            name_tree: Object::Null,
            metadata: Object::None,
            struct_tree_root: Object::None,
            outline: Object::None,
            acro_form: Object::None,
            needs_rendering: false,
            oc_properties: Object::None,
            viewer_prefs: Object::None,
        };

        let cat_dict = xref.get_catalog();
        if !cat_dict.is_dict() {
            error(
                ErrorCategory::SyntaxError,
                -1,
                &format!("Catalog object is wrong type ({})", cat_dict.get_type_name()),
            );
            c.ok = false;
            return c;
        }

        // read page tree
        if !c.read_page_tree(&cat_dict) {
            c.dests = Object::Null;
            c.name_tree = Object::Null;
            c.ok = false;
            return c;
        }

        // read named destination dictionary
        c.dests = cat_dict.dict_lookup("Dests");

        // read root of named destination tree
        let names = cat_dict.dict_lookup("Names");
        c.name_tree = if names.is_dict() { names.dict_lookup("Dests") } else { Object::Null };

        // get the metadata stream
        c.metadata = cat_dict.dict_lookup("Metadata");
        // get the structure tree root
        c.struct_tree_root = cat_dict.dict_lookup("StructTreeRoot");
        // get the outline dictionary
        c.outline = cat_dict.dict_lookup("Outlines");
        // get the AcroForm dictionary
        c.acro_form = cat_dict.dict_lookup("AcroForm");
        // get the NeedsRendering flag
        c.needs_rendering = matches!(cat_dict.dict_lookup("NeedsRendering"), Object::Bool(true));
        // get the OCProperties dictionary
        c.oc_properties = cat_dict.dict_lookup("OCProperties");
        // get the ViewerPreferences object
        c.viewer_prefs = cat_dict.dict_lookup_nf("ViewerPreferences");
        c
    }

    /// `isOk()`
    pub fn is_ok(&self) -> bool {
        self.ok
    }

    /// `getNumPages()`
    pub fn get_num_pages(&self) -> i32 {
        self.num_pages
    }

    /// `getPage(int i)` (1-based).  Returns `None` if `i` is out of range
    /// (undefined behaviour in xpdf).
    pub fn get_page(&self, i: i32) -> Option<Rc<Page>> {
        if i < 1 || i > self.num_pages {
            return None;
        }
        let loaded = self.state.borrow().pages[(i - 1) as usize].is_some();
        if !loaded {
            self.load_page(i);
        }
        self.state.borrow().pages[(i - 1) as usize].clone()
    }

    /// `getPageRef(int i)` (1-based).  A page that could not be loaded
    /// from the tree has the ref `{-1, -1}`.
    pub fn get_page_ref(&self, i: i32) -> Option<Ref> {
        if i < 1 || i > self.num_pages {
            return None;
        }
        let loaded = self.state.borrow().pages[(i - 1) as usize].is_some();
        if !loaded {
            self.load_page(i);
        }
        Some(self.state.borrow().page_refs[(i - 1) as usize])
    }

    /// `doneWithPage(int i)`
    pub fn done_with_page(&self, i: i32) {
        if i >= 1 && i <= self.num_pages {
            self.state.borrow_mut().pages[(i - 1) as usize] = None;
        }
    }

    /// `findPage(int num, int gen)`: page number, or 0 if not found.
    pub fn find_page(&self, num: i32, gen: i32) -> i32 {
        for i in 0..self.num_pages {
            let loaded = self.state.borrow().pages[i as usize].is_some();
            if !loaded {
                self.load_page(i + 1);
            }
            let r = self.state.borrow().page_refs[i as usize];
            if r.num == num && r.gen == gen {
                return i + 1;
            }
        }
        0
    }

    /// `findDest(GString *name)`.
    pub fn find_dest(&self, name: &[u8]) -> Option<LinkDest> {
        // try named destination dictionary then name tree
        let mut obj1 = Object::Null;
        let mut found = false;
        if self.dests.is_dict() {
            obj1 = self.dests.dict_lookup(name);
            if !obj1.is_null() {
                found = true;
            }
        }
        if !found && self.name_tree.is_dict() {
            let mut touched = vec![false; self.xref.get_num_objects().max(0) as usize];
            obj1 = self.find_dest_in_tree(&self.name_tree, &self.name_tree, name, &mut touched);
            if !obj1.is_null() {
                found = true;
            }
        }
        if !found {
            return None;
        }

        // construct LinkDest
        let mut dest = None;
        if let Object::Array(a) = &obj1 {
            dest = Some(LinkDest::new(a));
        } else if obj1.is_dict() {
            let d = obj1.dict_lookup("D");
            if let Object::Array(a) = &d {
                dest = Some(LinkDest::new(a));
            } else {
                error(ErrorCategory::SyntaxWarning, -1, "Bad named destination value");
            }
        } else {
            error(ErrorCategory::SyntaxWarning, -1, "Bad named destination value");
        }
        match dest {
            Some(d) if d.is_ok() => Some(d),
            _ => None,
        }
    }

    /// `findDestInTree(Object *treeRef, Object *tree, GString *name, ...)`.
    fn find_dest_in_tree(&self, tree_ref: &Object, tree: &Object, name: &[u8], touched: &mut [bool]) -> Object {
        // check for invalid reference
        if let Object::Ref(r) = tree_ref {
            if r.num < 0 || r.num >= self.xref.get_num_objects() {
                return Object::Null;
            }
            // check for a destination tree loop
            if touched[r.num as usize] {
                error(ErrorCategory::SyntaxError, -1, "Loop in destination name tree");
                return Object::Null;
            }
            touched[r.num as usize] = true;
        }

        // leaf node
        let names = tree.dict_lookup("Names");
        if names.is_array() {
            let mut i = 0;
            let n = names.array_get_length();
            while i < n {
                let name1 = names.array_get(i);
                if name1.is_string() {
                    let cmp = gstring_cmp(name, name1.get_string());
                    if cmp == 0 {
                        return names.array_get(i + 1);
                    } else if cmp < 0 {
                        return Object::Null;
                    }
                }
                i += 2;
            }
            return Object::Null;
        }

        // root or intermediate node
        let kids = tree.dict_lookup("Kids");
        if kids.is_array() {
            for i in 0..kids.array_get_length() {
                let kid_ref = kids.array_get_nf(i);
                let kid = kids.array_get(i);
                if kid.is_dict() {
                    let limits = kid.dict_lookup("Limits");
                    if limits.is_array() {
                        let low = limits.array_get(0);
                        if low.is_string() && gstring_cmp(name, low.get_string()) >= 0 {
                            let high = limits.array_get(1);
                            if high.is_string() && gstring_cmp(name, high.get_string()) <= 0 {
                                return self.find_dest_in_tree(&kid_ref, &kid, name, touched);
                            }
                        }
                    }
                }
            }
        }

        // name was outside of ranges of all kids
        Object::Null
    }

    /// `readPageTree(Object *catDict)`.
    fn read_page_tree(&mut self, cat_dict: &Object) -> bool {
        let top_pages_ref = cat_dict.dict_lookup_nf("Pages");
        if !top_pages_ref.is_ref() {
            error(
                ErrorCategory::SyntaxError,
                -1,
                &format!("Top-level pages reference is wrong type ({})", top_pages_ref.get_type_name()),
            );
            return false;
        }
        let top_pages_obj = top_pages_ref.fetch(Some(&self.xref));
        if !top_pages_obj.is_dict() {
            error(
                ErrorCategory::SyntaxError,
                -1,
                &format!("Top-level pages object is wrong type ({})", top_pages_obj.get_type_name()),
            );
            return false;
        }
        let mut num_pages;
        if let Object::Int(n) = top_pages_obj.dict_lookup("Count") {
            num_pages = n;
            if num_pages == 0 || num_pages > 50000 {
                // 1. Acrobat apparently scans the page tree if it sees a
                //    zero count.
                // 2. Absurdly large page counts result in very slow
                //    loading, because other code tries to fetch pages 1
                //    through n.
                // In both cases: ignore the given page count and scan the
                // tree instead.
                let mut touched = vec![false; self.xref.get_num_objects().max(0) as usize];
                num_pages = self.count_page_tree(&top_pages_ref, &mut touched);
            }
        } else {
            // assume we got a Page node instead of a Pages node
            num_pages = 1;
        }
        if num_pages < 0 {
            error(ErrorCategory::SyntaxError, -1, "Invalid page count");
            self.num_pages = 0;
            return false;
        }
        self.num_pages = num_pages;
        let mut st = self.state.borrow_mut();
        st.nodes.push(PageTreeNode {
            ref_: top_pages_ref.get_ref(),
            count: num_pages,
            parent: None,
            kids: None,
            attrs: None,
        });
        st.pages = vec![None; num_pages as usize];
        st.page_refs = vec![Ref { num: -1, gen: -1 }; num_pages as usize];
        true
    }

    /// `countPageTree(Object *pagesNodeRef, char *touchedObjs)`.
    fn count_page_tree(&self, pages_node_ref: &Object, touched: &mut [bool]) -> i32 {
        let num_objects = self.xref.get_num_objects();
        // check for invalid reference
        if let Object::Ref(r) = pages_node_ref {
            if r.num < 0 || r.num >= num_objects {
                return 0;
            }
        }

        // check for a page tree loop; fetch the node object
        let pages_node = if let Object::Ref(r) = pages_node_ref {
            if touched[r.num as usize] {
                error(ErrorCategory::SyntaxError, -1, "Loop in Pages tree");
                return 0;
            }
            touched[r.num as usize] = true;
            self.xref.fetch(r.num, r.gen)
        } else {
            pages_node_ref.clone()
        };

        // count the subtree
        let mut n: i32 = 0;
        if pages_node.is_dict() {
            let kids_ref = pages_node.dict_lookup_nf("Kids");
            let kids = match &kids_ref {
                Object::Ref(r) if r.num >= 0 && r.num < num_objects => {
                    if touched[r.num as usize] {
                        error(ErrorCategory::SyntaxError, -1, "Loop in Pages tree");
                        return 0;
                    }
                    touched[r.num as usize] = true;
                    self.xref.fetch(r.num, r.gen)
                }
                _ => kids_ref.clone(),
            };
            if kids.is_array() {
                for i in 0..kids.array_get_length() {
                    let kid = kids.array_get_nf(i);
                    let n2 = self.count_page_tree(&kid, touched);
                    if n2 < i32::MAX - n {
                        n += n2;
                    } else {
                        error(ErrorCategory::SyntaxError, -1, "Page tree contains too many pages");
                        n = i32::MAX;
                    }
                }
            } else {
                n = 1;
            }
        }
        n
    }

    /// `loadPage(int pg)`
    fn load_page(&self, pg: i32) {
        let mut st = self.state.borrow_mut();
        self.load_page2(&mut st, pg, pg - 1, 0);
    }

    /// `loadPage2(int pg, int relPg, PageTreeNode *node)`.
    fn load_page2(&self, st: &mut PageState, pg: i32, mut rel_pg: i32, node: usize) {
        let pgi = (pg - 1) as usize;
        if rel_pg >= st.nodes[node].count {
            error(ErrorCategory::SyntaxError, -1, "Internal error in page tree");
            st.pages[pgi] = Some(Rc::new(Page::new_empty(self.xref.clone(), pg)));
            return;
        }

        // if this node has not been filled in yet, it's either a leaf node
        // or an unread internal node
        if st.nodes[node].kids.is_none() {
            // check for a loop in the page tree
            let nref = st.nodes[node].ref_;
            let mut p = st.nodes[node].parent;
            while let Some(pi) = p {
                if st.nodes[pi].ref_ == nref {
                    error(ErrorCategory::SyntaxError, -1, "Loop in Pages tree");
                    st.pages[pgi] = Some(Rc::new(Page::new_empty(self.xref.clone(), pg)));
                    return;
                }
                p = st.nodes[pi].parent;
            }

            // fetch the Page/Pages object
            let page_obj = self.xref.fetch(nref.num, nref.gen);
            let page_dict = match &page_obj {
                Object::Dict(d) => d.clone(),
                _ => {
                    error(
                        ErrorCategory::SyntaxError,
                        -1,
                        &format!("Page tree object is wrong type ({})", page_obj.get_type_name()),
                    );
                    st.pages[pgi] = Some(Rc::new(Page::new_empty(self.xref.clone(), pg)));
                    return;
                }
            };

            // merge the PageAttrs
            let attrs = {
                let parent_attrs = st.nodes[node].parent.and_then(|pi| st.nodes[pi].attrs.as_ref());
                PageAttrs::new(parent_attrs, &page_dict, &self.xref)
            };

            // if "Kids" exists, it's an internal node
            let kids_obj = page_dict.lookup("Kids");
            if kids_obj.is_array() {
                // save the PageAttrs
                st.nodes[node].attrs = Some(attrs);

                // read the kids
                let mut kids = Vec::new();
                for i in 0..kids_obj.array_get_length() {
                    let kid_ref_obj = kids_obj.array_get_nf(i);
                    if let Object::Ref(kr) = kid_ref_obj {
                        let kid_obj = kid_ref_obj.fetch(Some(&self.xref));
                        if kid_obj.is_dict() {
                            let count = match kid_obj.dict_lookup("Count") {
                                Object::Int(c) => c,
                                _ => 1,
                            };
                            st.nodes.push(PageTreeNode {
                                ref_: kr,
                                count,
                                parent: Some(node),
                                kids: None,
                                attrs: None,
                            });
                            kids.push(st.nodes.len() - 1);
                        } else {
                            error(
                                ErrorCategory::SyntaxError,
                                -1,
                                &format!("Page tree object is wrong type ({})", kid_obj.get_type_name()),
                            );
                        }
                    } else {
                        error(
                            ErrorCategory::SyntaxError,
                            -1,
                            &format!("Page tree reference is wrong type ({})", kid_ref_obj.get_type_name()),
                        );
                    }
                }
                st.nodes[node].kids = Some(kids);
            } else {
                // create the Page object
                st.page_refs[pgi] = nref;
                let page = Page::new(self.xref.clone(), pg, &page_dict, attrs);
                let page = if page.is_ok() { page } else { Page::new_empty(self.xref.clone(), pg) };
                st.pages[pgi] = Some(Rc::new(page));
            }
        }

        // recursively descend the tree
        if let Some(kids) = st.nodes[node].kids.clone() {
            let mut i = 0;
            while i < kids.len() {
                let kid_node = kids[i];
                if rel_pg < st.nodes[kid_node].count {
                    self.load_page2(st, pg, rel_pg, kid_node);
                    break;
                }
                rel_pg -= st.nodes[kid_node].count;
                i += 1;
            }

            // this will only happen if the page tree is invalid
            // (i.e., parent count > sum of children counts)
            if i == kids.len() {
                error(ErrorCategory::SyntaxError, -1, "Invalid page count in page tree");
                st.pages[pgi] = Some(Rc::new(Page::new_empty(self.xref.clone(), pg)));
            }
        }
    }

    /// `getDests()`
    pub fn get_dests(&self) -> &Object {
        &self.dests
    }
    /// `getNameTree()`
    pub fn get_name_tree(&self) -> &Object {
        &self.name_tree
    }
    /// The catalog's /Metadata (fetched).
    pub fn get_metadata(&self) -> &Object {
        &self.metadata
    }
    /// `getStructTreeRoot()`
    pub fn get_struct_tree_root(&self) -> &Object {
        &self.struct_tree_root
    }
    /// `getOutline()`
    pub fn get_outline(&self) -> &Object {
        &self.outline
    }
    /// `getAcroForm()`
    pub fn get_acro_form(&self) -> &Object {
        &self.acro_form
    }
    /// `getNeedsRendering()`
    pub fn get_needs_rendering(&self) -> bool {
        self.needs_rendering
    }
    /// `getOCProperties()`
    pub fn get_oc_properties(&self) -> &Object {
        &self.oc_properties
    }
    /// `getViewerPreferences()`
    pub fn get_viewer_preferences(&self) -> &Object {
        &self.viewer_prefs
    }
    /// `readMetadata()`: contents of the metadata stream.
    pub fn read_metadata(&self) -> Option<Vec<u8>> {
        let s = self.metadata.get_stream()?;
        if !s.get_dict().lookup("Subtype").is_name_of("XML") {
            let st = s.get_dict().lookup("Subtype");
            error(
                ErrorCategory::SyntaxWarning,
                -1,
                &format!(
                    "Unknown Metadata type: '{}'",
                    if st.is_name() { st.get_name_str() } else { "???".to_string() }
                ),
            );
        }
        s.get_decoded_data().ok()
    }
}

/// `GString::cmp(GString *str)`: byte-wise compare, then length.
pub(crate) fn gstring_cmp(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len().min(b.len());
    for i in 0..n {
        let x = a[i] as i32 - b[i] as i32;
        if x != 0 {
            return x;
        }
    }
    a.len() as i32 - b.len() as i32
}
