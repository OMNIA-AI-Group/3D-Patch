#include <algorithm>
#include <chrono>
// Passerelle Rhino 3DM (openNURBS, McNeel, licence MIT) pour 3D-Patch.
// Import : maillages + maillages de rendu des NURBS/extrusions + blocs (instances), unité -> mm,
// couleur d'objet/calque. Export : un maillage en millimètres.
#include "opennurbs_public.h"
// OpenCASCADE : maillage des NURBS sans maillage de rendu (fichiers MoI, Rhino « petit fichier »)
#include <Geom_BSplineSurface.hxx>
#include <Geom2d_BSplineCurve.hxx>
#include <TColgp_Array2OfPnt.hxx>
#include <TColgp_Array1OfPnt2d.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColStd_Array2OfReal.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepLib.hxx>
#include <ShapeFix_Face.hxx>
#include <ShapeFix_Wire.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <Poly_Triangulation.hxx>
#include <Standard_Failure.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <gp_GTrsf.hxx>
#include <TopTools_DataMapOfShapeInteger.hxx>
#include <vector>
#include <cmath>
#include <string>
#include <cstring>
#include <cstdio>
#include <cstdlib>

// chronométrage des étapes (PATCH3D_3DMDBG)
static double horloge() { return std::chrono::duration<double>(std::chrono::steady_clock::now().time_since_epoch()).count(); }
static void etape(const char* nom, double& t) { double n = horloge(); if (std::getenv("PATCH3D_3DMDBG")) std::fprintf(stderr, "  3DM %-22s %.2f s\n", nom, n - t); t = n; }
static void err(char* e, int n, const std::string& s) { if (e && n > 0) { std::strncpy(e, s.c_str(), n - 1); e[n - 1] = 0; } }
static void init() { static bool ok = false; if (!ok) { ON::Begin(); ok = true; } }

struct FaceCol { TopoDS_Face f; ON_Color col; int obj; }; // obj : objet d'origine
struct Sortie { std::vector<float> v, c; int sans_maillage = 0; std::vector<FaceCol> faces; int n_obj = 0; };

static void ajoute_maillage(Sortie& o, const ON_Mesh* m, const ON_Xform& x, ON_Color col) {
    if (!m) return;
    const float r = col.FractionRed(), g = col.FractionGreen(), b = col.FractionBlue();
    auto tri = [&](int a, int bb, int cc) {
        ON_3dPoint p[3] = { x * m->Vertex(a), x * m->Vertex(bb), x * m->Vertex(cc) };
        ON_3dVector n = ON_CrossProduct(p[1] - p[0], p[2] - p[0]); n.Unitize();
        for (int k = 0; k < 3; ++k) {
            o.v.insert(o.v.end(), { (float)p[k].x, (float)p[k].y, (float)p[k].z, (float)n.x, (float)n.y, (float)n.z });
            o.c.insert(o.c.end(), { r, g, b });
        }
    };
    for (int f = 0; f < m->m_F.Count(); ++f) {
        const ON_MeshFace& F = m->m_F[f];
        tri(F.vi[0], F.vi[1], F.vi[2]);
        if (F.vi[2] != F.vi[3]) tri(F.vi[0], F.vi[2], F.vi[3]); // quadrangle
    }
}

// nœuds openNURBS (ordre+cv-2 valeurs) -> nœuds OCCT (valeurs distinctes + multiplicités),
// en ajoutant les nœuds « superflus » de début et de fin qu'openNURBS omet
static void noeuds(const double* k, int n, TColStd_Array1OfReal& K, TColStd_Array1OfInteger& M, int& nk) {
    std::vector<double> f; f.push_back(k[0]); for (int i = 0; i < n; ++i) f.push_back(k[i]); f.push_back(k[n - 1]);
    std::vector<double> u; std::vector<int> m;
    for (double x : f) { if (!u.empty() && std::fabs(x - u.back()) <= 1e-12 * (1.0 + std::fabs(x))) m.back()++; else { u.push_back(x); m.push_back(1); } }
    nk = (int)u.size(); K.Resize(1, nk, false); M.Resize(1, nk, false);
    for (int i = 0; i < nk; ++i) { K(i + 1) = u[i]; M(i + 1) = m[i]; }
}
static Handle(Geom_BSplineSurface) vers_occt(const ON_NurbsSurface& ns) {
    const int nu = ns.CVCount(0), nv = ns.CVCount(1);
    TColgp_Array2OfPnt P(1, nu, 1, nv); TColStd_Array2OfReal W(1, nu, 1, nv);
    for (int i = 0; i < nu; ++i) for (int j = 0; j < nv; ++j) {
        ON_3dPoint q; ns.GetCV(i, j, q); P(i + 1, j + 1) = gp_Pnt(q.x, q.y, q.z); W(i + 1, j + 1) = ns.IsRational() ? ns.Weight(i, j) : 1.0;
    }
    TColStd_Array1OfReal KU(1, 1), KV(1, 1); TColStd_Array1OfInteger MU(1, 1), MV(1, 1); int a, b;
    noeuds(ns.m_knot[0], ns.KnotCount(0), KU, MU, a); noeuds(ns.m_knot[1], ns.KnotCount(1), KV, MV, b);
    if (ns.IsRational()) return new Geom_BSplineSurface(P, W, KU, KV, MU, MV, ns.Order(0) - 1, ns.Order(1) - 1);
    return new Geom_BSplineSurface(P, KU, KV, MU, MV, ns.Order(0) - 1, ns.Order(1) - 1);
}
static Handle(Geom2d_BSplineCurve) vers_occt2d(const ON_NurbsCurve& nc) {
    const int n = nc.CVCount();
    TColgp_Array1OfPnt2d P(1, n); TColStd_Array1OfReal W(1, n);
    for (int i = 0; i < n; ++i) { ON_3dPoint q; nc.GetCV(i, q); P(i + 1) = gp_Pnt2d(q.x, q.y); W(i + 1) = nc.IsRational() ? nc.Weight(i) : 1.0; }
    TColStd_Array1OfReal K(1, 1); TColStd_Array1OfInteger M(1, 1); int a;
    noeuds(nc.m_knot, nc.KnotCount(), K, M, a);
    if (nc.IsRational()) return new Geom2d_BSplineCurve(P, W, K, M, nc.Order() - 1);
    return new Geom2d_BSplineCurve(P, K, M, nc.Order() - 1);
}
// Face B-rep openNURBS -> face OpenCASCADE (surface + boucles de découpe en 2D)
static bool face_occt(const ON_Brep& br, const ON_BrepFace& f, TopoDS_Face& out) {
    ON_NurbsSurface ns;
    const ON_Surface* srf = f.SurfaceOf();
    if (!srf || !srf->GetNurbForm(ns)) return false;
    Handle(Geom_BSplineSurface) S = vers_occt(ns);
    BRepBuilderAPI_MakeFace mf;
    bool ext = false;
    for (int li = 0; li < f.m_li.Count(); ++li) {
        const ON_BrepLoop& L = br.m_L[f.m_li[li]];
        BRepBuilderAPI_MakeWire mw;
        for (int ti = 0; ti < L.m_ti.Count(); ++ti) {
            const ON_BrepTrim& T = br.m_T[L.m_ti[ti]];
            ON_NurbsCurve nc;
            if (!T.GetNurbForm(nc)) continue;
            BRepBuilderAPI_MakeEdge me(vers_occt2d(nc), S);
            if (me.IsDone()) mw.Add(me.Edge());
        }
        if (!mw.IsDone()) continue;
        TopoDS_Wire w = mw.Wire();
        if (L.m_type == ON_BrepLoop::outer && !ext) { mf = BRepBuilderAPI_MakeFace(S, w, Standard_True); ext = mf.IsDone(); }
        else if (ext) mf.Add(w);
    }
    if (!ext) { // pas de boucle exploitable : surface entière
        mf = BRepBuilderAPI_MakeFace(S, 1e-6); if (!mf.IsDone()) return false;
    }
    TopoDS_Face fc = mf.Face();
    BRepLib::BuildCurves3d(fc, 1e-4);
    // corrections utiles seulement : les contours Rhino/MoI sont propres, la recherche
    // d'auto-intersections coûtait ~45 % du temps d'import
    ShapeFix_Face sf(fc);
    sf.FixWireTool()->FixSelfIntersectionMode() = 0;
    sf.FixWireTool()->FixIntersectingEdgesMode() = 0;
    sf.FixWireTool()->FixNonAdjacentIntersectingEdgesMode() = 0;
    sf.Perform(); fc = sf.Face();
    if (f.m_bRev) fc.Reverse();
    out = fc;
    return true;
}
// B-rep sans maillage de rendu : ses faces sont converties, placées (bloc + unité -> mm) et mises
// de côté ; elles seront recousues TOUTES ENSEMBLE (surfaces séparées d'une même pièce, typique MoI)
static bool collecte_brep(Sortie& o, const ON_Brep& br, const ON_Xform& x, ON_Color col) {
    try {
        gp_Trsf t; bool simil = true;
        try { t.SetValues(x.m_xform[0][0], x.m_xform[0][1], x.m_xform[0][2], x.m_xform[0][3],
                          x.m_xform[1][0], x.m_xform[1][1], x.m_xform[1][2], x.m_xform[1][3],
                          x.m_xform[2][0], x.m_xform[2][1], x.m_xform[2][2], x.m_xform[2][3]); }
        catch (const Standard_Failure&) { simil = false; }
        int n = 0, id = o.n_obj++;
        for (int i = 0; i < br.m_F.Count(); ++i) {
            TopoDS_Face fc; if (!face_occt(br, br.m_F[i], fc)) continue;
            TopoDS_Shape sh;
            if (simil) sh = BRepBuilderAPI_Transform(fc, t, Standard_True).Shape();
            else {
                gp_GTrsf g; for (int r = 1; r <= 3; ++r) { for (int c = 1; c <= 3; ++c) g.SetValue(r, c, x.m_xform[r - 1][c - 1]); g.SetValue(r, 4, x.m_xform[r - 1][3]); }
                sh = BRepBuilderAPI_GTransform(fc, g, Standard_True).Shape();
            }
            for (TopExp_Explorer ex(sh, TopAbs_FACE); ex.More(); ex.Next()) { o.faces.push_back({ TopoDS::Face(ex.Current()), col, id }); n++; }
        }
        return n > 0;
    } catch (const Standard_Failure&) { return false; }
}
// Couture + maillage + couleurs par face d'origine (la fusion des arêtes de la couture coûte l'essentiel du
// temps d'ouverture d'un 3DM : résultat gardé en cache côté Rust, voir cao_en_cache)
static void maille_objet(const std::vector<const FaceCol*>& faces, double defl_mm, std::vector<float>& v, std::vector<float>& c, int& rate) {
    try {
        TopoDS_Shape sh;
        TopTools_DataMapOfShapeInteger coul;
        if (faces.size() == 1) { sh = faces[0]->f; coul.Bind(sh, 0); }
        else {
            // NB : l'essentiel du temps est la fusion des arêtes (Merging -> Approx_SameParameter), qu'aucune
            // option n'évite ; le résultat est donc gardé en cache côté Rust (cao_en_cache)
            BRepBuilderAPI_Sewing sew(0.05, Standard_True, Standard_True, Standard_False, Standard_False); // 0,05 mm, sans découpage (moins d'erreurs)
            sew.SetSameParameterMode(Standard_False); // recalage fin des arêtes inutile pour mailler (~30 % du temps)
            for (auto* fc : faces) sew.Add(fc->f);
            sew.Perform();
            sh = sew.SewedShape();
            for (size_t i = 0; i < faces.size(); ++i) {
                const TopoDS_Shape& f0 = faces[i]->f;
                TopoDS_Shape f1 = sew.IsModified(f0) ? sew.Modified(f0) : f0;
                for (TopExp_Explorer ex(f1, TopAbs_FACE); ex.More(); ex.Next()) coul.Bind(ex.Current(), (int)i);
            }
        }
        BRepMesh_IncrementalMesh mesher(sh, defl_mm, Standard_False, 0.1, Standard_True); mesher.Perform();
        for (TopExp_Explorer ex(sh, TopAbs_FACE); ex.More(); ex.Next()) {
            TopoDS_Face face = TopoDS::Face(ex.Current()); TopLoc_Location loc;
            Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
            if (tri.IsNull()) continue;
            ON_Color col(237, 204, 84);
            if (coul.IsBound(face)) col = faces[coul.Find(face)]->col;
            else { TopoDS_Face fr = face; fr.Orientation(TopAbs_FORWARD); if (coul.IsBound(fr)) col = faces[coul.Find(fr)]->col; }
            const float r = col.FractionRed(), g = col.FractionGreen(), b = col.FractionBlue();
            const gp_Trsf& tr = loc.Transformation(); bool rev = face.Orientation() == TopAbs_REVERSED;
            for (int i = 1; i <= tri->NbTriangles(); ++i) {
                Standard_Integer ia, ib, ic; tri->Triangle(i).Get(ia, ib, ic); if (rev) std::swap(ib, ic);
                gp_Pnt p[3] = { tri->Node(ia).Transformed(tr), tri->Node(ib).Transformed(tr), tri->Node(ic).Transformed(tr) };
                if (ia == ib || ib == ic || ia == ic || p[0].IsEqual(p[1], 1e-9) || p[1].IsEqual(p[2], 1e-9) || p[0].IsEqual(p[2], 1e-9)) continue; // dégénéré
                gp_Vec n = gp_Vec(p[0], p[1]).Crossed(gp_Vec(p[0], p[2])); double l = n.Magnitude(); if (l > 1e-20) n /= l;
                for (int k = 0; k < 3; ++k) {
                    v.insert(v.end(), { (float)p[k].X(), (float)p[k].Y(), (float)p[k].Z(), (float)n.X(), (float)n.Y(), (float)n.Z() });
                    c.insert(c.end(), { r, g, b });
                }
            }
        }
    } catch (const Standard_Failure&) { rate += (int)faces.size(); }
}
// Toutes les faces NURBS du fichier cousues ENSEMBLE : des objets Rhino qui se touchent sont soudés entre eux
// (coudre objet par objet laissait des trous entre objets voisins : boite.3dm 58 trous au lieu de 8)
static void maille_faces(Sortie& o, double defl_mm) {
    if (o.faces.empty()) return;
    double t = horloge();
    std::vector<const FaceCol*> toutes;
    for (auto& fc : o.faces) toutes.push_back(&fc);
    maille_objet(toutes, defl_mm, o.v, o.c, o.sans_maillage);
    etape("couture + maillage", t);
    o.faces.clear();
}

static void ajoute_geometrie(Sortie& o, const ONX_Model& model, const ON_Geometry* g, const ON_Xform& x, ON_Color col, int prof) {
    if (!g || prof > 16) return;
    if (const ON_Mesh* m = ON_Mesh::Cast(g)) { ajoute_maillage(o, m, x, col); return; }
    if (const ON_Brep* br = ON_Brep::Cast(g)) {
        // maillages de rendu enregistrés par Rhino si présents sur TOUTES les faces, sinon maillage OCCT
        bool tous = br->m_F.Count() > 0;
        for (int i = 0; i < br->m_F.Count() && tous; ++i) if (!br->m_F[i].Mesh(ON::render_mesh) && !br->m_F[i].Mesh(ON::any_mesh)) tous = false;
        if (tous) {
            for (int i = 0; i < br->m_F.Count(); ++i) { const ON_Mesh* m = br->m_F[i].Mesh(ON::render_mesh); if (!m) m = br->m_F[i].Mesh(ON::any_mesh); ajoute_maillage(o, m, x, col); }
        } else if (!collecte_brep(o, *br, x, col)) o.sans_maillage++;
        return;
    }
    if (const ON_Extrusion* ex = ON_Extrusion::Cast(g)) {
        const ON_Mesh* m = ex->m_mesh_cache.Mesh(ON::render_mesh);
        if (m) { ajoute_maillage(o, m, x, col); return; }
        ON_Brep* br = ex->BrepForm();
        if (!br || !collecte_brep(o, *br, x, col)) o.sans_maillage++;
        delete br;
        return;
    }
    if (const ON_InstanceRef* ir = ON_InstanceRef::Cast(g)) {
        ON_ModelComponentReference ref = model.ComponentFromId(ON_ModelComponent::Type::InstanceDefinition, ir->m_instance_definition_uuid);
        const ON_InstanceDefinition* idef = ON_InstanceDefinition::Cast(ref.ModelComponent());
        if (!idef) return;
        const ON_Xform xx = x * ir->m_xform;
        const ON_SimpleArray<ON_UUID>& ids = idef->InstanceGeometryIdList();
        for (int i = 0; i < ids.Count(); ++i) {
            const ON_ModelGeometryComponent& mg = model.ModelGeometryComponentFromId(ids[i]);
            ajoute_geometrie(o, model, mg.Geometry(nullptr), xx, col, prof + 1);
        }
        return;
    }
    o.sans_maillage++; // SubD, courbes, etc.
}

extern "C" int on_import(const char* path, float** verts, size_t* nv, float** cols, int* sans_maillage, char* e, int en) {
    init();
    ONX_Model model; ON_TextLog log;
    double t = horloge();
    if (!model.Read(path, &log)) { err(e, en, "fichier 3DM illisible"); return 1; }
    etape("lecture openNURBS", t);
    const double s = ON::UnitScale(model.m_settings.m_ModelUnitsAndTolerances.m_unit_system, ON::LengthUnitSystem::Millimeters);
    ON_Xform ech(ON_Xform::DiagonalTransformation(s > 0 ? s : 1.0));
    Sortie o;
    ONX_ModelComponentIterator it(model, ON_ModelComponent::Type::ModelGeometry);
    for (const ON_ModelComponent* mc = it.FirstComponent(); mc; mc = it.NextComponent()) {
        const ON_ModelGeometryComponent* mg = ON_ModelGeometryComponent::Cast(mc);
        if (!mg) continue;
        const ON_3dmObjectAttributes* at = mg->Attributes(nullptr);
        if (at && at->IsInstanceDefinitionObject()) continue; // géométrie de bloc : dessinée via ses instances
        if (at && !at->IsVisible()) continue;
        ON_Color col(237, 204, 84);
        if (at) {
            if (at->ColorSource() == ON::color_from_object) col = at->m_color;
            else {
                const ON_Layer* ly = ON_Layer::Cast(model.LayerFromIndex(at->m_layer_index).ModelComponent());
                if (ly) col = ly->Color();
            }
        }
        // noir = couleur par défaut des calques (MoI) : pas une vraie couleur -> jaune de l'appli
        if (col.Red() < 16 && col.Green() < 16 && col.Blue() < 16) col = ON_Color(237, 204, 84);
        ajoute_geometrie(o, model, mg->Geometry(nullptr), ech, col, 0);
    }
    etape("conversion NURBS->OCCT", t);
    if (std::getenv("PATCH3D_3DMDBG")) std::fprintf(stderr, "  3DM %zu faces NURBS, %zu sommets déjà maillés\n", o.faces.size(), o.v.size() / 6);
    maille_faces(o, 0.02); // faces déjà en millimètres : précision 0,02 mm (comme occt::DEFLEXION)
    *sans_maillage = o.sans_maillage;
    if (o.v.empty()) { err(e, en, o.sans_maillage ? "objets NURBS sans maillage de rendu : dans Rhino, enregistrer AVEC les maillages (ou exporter en STEP)" : "aucun objet 3D dans le fichier"); return 2; }
    *nv = o.v.size();
    *verts = (float*)std::malloc(o.v.size() * sizeof(float)); std::memcpy(*verts, o.v.data(), o.v.size() * sizeof(float));
    *cols = (float*)std::malloc(o.c.size() * sizeof(float)); std::memcpy(*cols, o.c.data(), o.c.size() * sizeof(float));
    return 0;
}

extern "C" void on_free(float* p) { std::free(p); }

// Faces planes issues du maillage : arêtes exactes => tolérances nulles
static void fix_tol(ON_Brep* b) {
    for (int i = 0; i < b->m_E.Count(); ++i) b->m_E[i].m_tolerance = 0.0;
    for (int i = 0; i < b->m_V.Count(); ++i) b->m_V[i].m_tolerance = 0.0;
    for (int i = 0; i < b->m_T.Count(); ++i) { b->m_T[i].m_tolerance[0] = 0.0; b->m_T[i].m_tolerance[1] = 0.0; }
    b->SetTolerancesBoxesAndFlags(true);
}

// version : 5 = Rhino 5 / MoI (polysurface), 8 = Rhino 8 (maillage natif)
extern "C" int on_export(const char* path, const float* verts, size_t ntri, const float* cols, int version, char* e, int en) {
    init();
    ONX_Model model;
    model.m_properties.m_Application.m_application_name = L"3D-Patch";
    model.m_settings.m_ModelUnitsAndTolerances.m_unit_system = ON_UnitSystem(ON::LengthUnitSystem::Millimeters);
    ON_Mesh* m = new ON_Mesh((int)ntri, (int)(ntri * 3), true, false);
    for (size_t t = 0; t < ntri; ++t) {
        for (int k = 0; k < 3; ++k) {
            const float* s = verts + (t * 3 + k) * 6;
            m->m_V.Append(ON_3fPoint(s[0], s[1], s[2]));
            m->m_N.Append(ON_3fVector(s[3], s[4], s[5]));
            if (cols) { const float* c = cols + (t * 3 + k) * 3; m->m_C.Append(ON_Color((int)(c[0] * 255), (int)(c[1] * 255), (int)(c[2] * 255))); }
        }
        m->SetTriangle((int)t, (int)(t * 3), (int)(t * 3 + 1), (int)(t * 3 + 2));
    }
    m->CombineIdenticalVertices(true, true);
    m->ComputeFaceNormals();
    ON_3dmObjectAttributes* at = new ON_3dmObjectAttributes();
    at->m_name = L"3D-Patch";
    // MoI 3D (et tout modeleur NURBS) ignore les maillages : on écrit une polysurface,
    // faces planes regroupées (n-gones coplanaires). Au-delà d'une taille raisonnable : maillage.
    ON_Brep* brep = nullptr;
    if (version == 5 && ntri <= 200000) {
        const double tol = 1e-3; // mm
        m->AddPlanarNgons(nullptr, tol, 3, 2, true);
        brep = ON_BrepFromMeshWithNgons(m->Topology(), true, true, tol);
        if (brep) fix_tol(brep); // tolérances d'arêtes non calculées par openNURBS (réservé à Rhino)
        if (std::getenv("PATCH3D_3DMDBG")) std::fprintf(stderr, "3DM export : ngons %u, brep %s, faces %d, valide %d\n", m->NgonUnsignedCount(), brep ? "oui" : "non", brep ? brep->m_F.Count() : 0, brep ? (int)brep->IsValid() : 0);
        if (brep && !brep->IsValid()) {
            if (std::getenv("PATCH3D_3DMDBG")) { ON_wString w; ON_TextLog tl(w); brep->IsValid(&tl); std::fprintf(stderr, "%.600ls\n", static_cast<const wchar_t*>(w)); }
            delete brep;
            // repli : un triangle = une face plane détourée (toujours valide)
            ON_Mesh tri(*m); tri.RemoveAllNgons();
            brep = ON_BrepFromMesh(tri.Topology(), true);
            if (brep) fix_tol(brep);
            if (std::getenv("PATCH3D_3DMDBG")) std::fprintf(stderr, "repli triangles : faces %d valide %d\n", brep ? brep->m_F.Count() : 0, brep ? (int)brep->IsValid() : 0);
            if (brep && !brep->IsValid()) { delete brep; brep = nullptr; }
        }
    }
    if (brep) {
        if (cols && ntri) {
            double r = 0, g = 0, b = 0;
            for (size_t i = 0; i < ntri * 3; ++i) { r += cols[i * 3]; g += cols[i * 3 + 1]; b += cols[i * 3 + 2]; }
            const double k = 255.0 / (ntri * 3);
            at->m_color = ON_Color((int)(r * k), (int)(g * k), (int)(b * k));
            at->SetColorSource(ON::color_from_object);
        }
        delete m;
        model.AddManagedModelGeometryComponent(brep, at);
    } else {
        model.AddManagedModelGeometryComponent(m, at);
    }
    ON_TextLog log;
    // 50 = Rhino 5 : lisible par MoI 3D (qui refuse les 3DM v6+) et Rhino 5 à 8 ; 0 = version courante (8)
    if (!model.Write(path, version == 5 ? 50 : 0, &log)) { err(e, en, "écriture 3DM impossible"); return 1; }
    return 0;
}
