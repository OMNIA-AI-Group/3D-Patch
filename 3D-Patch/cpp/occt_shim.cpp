// Passerelle OCCT -> Rust pour le viewer : crée une forme (primitive) et renvoie sa
// tessellation (triangles + normales à plat) prête pour OpenGL. Convention : Z vers le haut,
// la forme repose sur le plan Z=0.
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopoDS_Edge.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <Poly_Triangulation.hxx>
#include <TopLoc_Location.hxx>
#include <TopAbs_Orientation.hxx>
#include <gp_Ax2.hxx>
#include <gp_Pnt.hxx>
#include <gp_Dir.hxx>
#include <Standard_Failure.hxx>
#include <STEPControl_Reader.hxx>
#include <IGESControl_Reader.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <vector>
#include <cmath>
#include <cstdlib>
#include <cstring>

// Tessellation d'une forme en tableau entrelacé [x y z nx ny nz] * 3 sommets * triangles.
static int tessellate(const TopoDS_Shape& shape, double defl, float** out, int* out_nfloats) {
    // flèche linéaire `defl` (mm) ET angulaire 0,1 rad (~6°) : courbes fidèles même sur les petites pièces
    BRepMesh_IncrementalMesh mesher(shape, defl, Standard_False, 0.1, Standard_True);
    mesher.Perform();
    std::vector<float> buf;
    for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
        TopoDS_Face face = TopoDS::Face(ex.Current());
        TopLoc_Location loc;
        Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
        if (tri.IsNull()) continue;
        const gp_Trsf& trsf = loc.Transformation();
        bool reversed = (face.Orientation() == TopAbs_REVERSED);
        for (int i = 1; i <= tri->NbTriangles(); ++i) {
            Standard_Integer a, b, c;
            tri->Triangle(i).Get(a, b, c);
            if (reversed) std::swap(b, c);
            gp_Pnt p1 = tri->Node(a).Transformed(trsf);
            gp_Pnt p2 = tri->Node(b).Transformed(trsf);
            gp_Pnt p3 = tri->Node(c).Transformed(trsf);
            // triangle dégénéré (deux sommets confondus, ex. pôle d'un congé de coin) : ignoré,
            // sinon il apparaît comme une arête en erreur sur une pièce pourtant parfaite
            if (a == b || b == c || a == c || p1.IsEqual(p2, 1e-9) || p2.IsEqual(p3, 1e-9) || p1.IsEqual(p3, 1e-9)) continue;
            // normale à plat du triangle
            double ux = p2.X()-p1.X(), uy = p2.Y()-p1.Y(), uz = p2.Z()-p1.Z();
            double vx = p3.X()-p1.X(), vy = p3.Y()-p1.Y(), vz = p3.Z()-p1.Z();
            double nx = uy*vz - uz*vy, ny = uz*vx - ux*vz, nz = ux*vy - uy*vx;
            double len = std::sqrt(nx*nx + ny*ny + nz*nz); if (len < 1e-12) len = 1.0;
            nx /= len; ny /= len; nz /= len;
            const gp_Pnt* ps[3] = { &p1, &p2, &p3 };
            for (int k = 0; k < 3; ++k) {
                buf.push_back((float)ps[k]->X()); buf.push_back((float)ps[k]->Y()); buf.push_back((float)ps[k]->Z());
                buf.push_back((float)nx); buf.push_back((float)ny); buf.push_back((float)nz);
            }
        }
    }
    float* arr = (float*)std::malloc(buf.size() * sizeof(float));
    if (!arr) return 1;
    std::memcpy(arr, buf.data(), buf.size() * sizeof(float));
    *out = arr;
    *out_nfloats = (int)buf.size();
    return 0;
}

extern "C" void occt_free(float* p) { std::free(p); }

// Primitives, posées sur Z=0. kind : 0=boîte,1=sphère,2=cylindre,3=cône,4=tore.
extern "C" int occt_make_primitive(int kind, double a, double b, double c, double defl,
                                    float** out, int* out_nfloats) {
    try {
        TopoDS_Shape s;
        switch (kind) {
            case 0: s = BRepPrimAPI_MakeBox(a, b, c).Shape(); break;                       // a,b,c = dx,dy,dz
            case 1: s = BRepPrimAPI_MakeSphere(gp_Ax2(gp_Pnt(0,0,a), gp_Dir(0,0,1)), a).Shape(); break; // a = rayon, centre à z=r
            case 2: s = BRepPrimAPI_MakeCylinder(a, b).Shape(); break;                     // a=rayon, b=hauteur
            case 3: s = BRepPrimAPI_MakeCone(a, b, c).Shape(); break;                      // a=r1, b=r2, c=hauteur
            case 4: s = BRepPrimAPI_MakeTorus(gp_Ax2(gp_Pnt(0,0,b), gp_Dir(0,0,1)), a, b).Shape(); break; // a=R, b=r
            default: return 2;
        }
        return tessellate(s, defl, out, out_nfloats);
    } catch (Standard_Failure&) { return 10; } catch (...) { return 11; }
}

// Objet de démonstration : boîte avec congés sur toutes les arêtes (vitrine OCCT).
extern "C" int occt_make_demo(double dx, double dy, double dz, double r, double defl,
                              float** out, int* out_nfloats) {
    try {
        TopoDS_Shape box = BRepPrimAPI_MakeBox(dx, dy, dz).Shape();
        BRepFilletAPI_MakeFillet fil(box);
        for (TopExp_Explorer ex(box, TopAbs_EDGE); ex.More(); ex.Next()) {
            fil.Add(r, TopoDS::Edge(ex.Current()));
        }
        fil.Build();
        if (!fil.IsDone()) return 12;
        return tessellate(fil.Shape(), defl, out, out_nfloats);
    } catch (Standard_Failure&) { return 10; } catch (...) { return 11; }
}

// Import d'un fichier CAO neutre (STEP ou IGES) -> tessellation directe.
extern "C" int occt_import_cad(const char* path, int is_iges, double defl,
                               float** out, int* out_nfloats) {
    try {
        TopoDS_Shape shape;
        if (is_iges) {
            IGESControl_Reader r;
            if (r.ReadFile(path) != IFSelect_RetDone) return 40;
            r.TransferRoots();
            shape = r.OneShape();
        } else {
            STEPControl_Reader r;
            if (r.ReadFile(path) != IFSelect_RetDone) return 40;
            r.TransferRoots();
            shape = r.OneShape();
        }
        if (shape.IsNull()) return 41;
        return tessellate(shape, defl, out, out_nfloats);
    } catch (Standard_Failure&) { return 42; } catch (...) { return 43; }
}

// Import d'un fichier BREP OpenCASCADE (contenu dans les .FCStd de FreeCAD) -> tessellation.
extern "C" int occt_import_brep(const char* path, double defl,
                                float** out, int* out_nfloats) {
    try {
        TopoDS_Shape shape;
        BRep_Builder b;
        if (!BRepTools::Read(shape, path, b)) return 40;
        if (shape.IsNull()) return 41;
        return tessellate(shape, defl, out, out_nfloats);
    } catch (Standard_Failure&) { return 42; } catch (...) { return 43; }
}
