//! A design saves `cut_list`: its cut list's item numbers (`numbers`) and descriptions
//! (`descriptions`), each by the item's grouping key. An older design has none and numbers its
//! items in list order, as before. The last release opens a design with this change with a warning
//! about the newer format and numbers its cut list in list order.
