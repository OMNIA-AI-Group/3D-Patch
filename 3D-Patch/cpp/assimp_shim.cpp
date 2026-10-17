// Passerelle Assimp (formats 3DS, DAE/Collada, FBX, glTF/GLB, X3D…) pour 3D-Patch.
// Import : triangles [pos nrm] à plat (transformations des nœuds appliquées) + couleur par sommet.
// Export : un objet maillé (pos + normales) dans le format demandé (identifiant Assimp).
#include <assimp/Importer.hpp>
#include <assimp/Exporter.hpp>
#include <assimp/scene.h>
#include <assimp/postprocess.h>
#include <cstdlib>
#include <cctype>
#include <cstring>
#include <vector>
#include <string>

static void err(char* e, int n, const std::string& s) { if (e && n > 0) { std::strncpy(e, s.c_str(), n - 1); e[n - 1] = 0; } }

extern "C" int as_import(const char* path, float** verts, size_t* nv, float** cols, int* has_col, char* e, int en) {
    Assimp::Importer imp;
    const aiScene* sc = imp.ReadFile(path, aiProcess_Triangulate | aiProcess_PreTransformVertices |
        aiProcess_GenSmoothNormals | aiProcess_SortByPType | aiProcess_FindDegenerates);
    if (!sc || !sc->mRootNode) { err(e, en, imp.GetErrorString()); return 1; }
    // unité -> millimètres : glTF/GLB/X3D/DAE en mètres ; FBX selon UnitScaleFactor (en cm, défaut 1 cm)
    std::string ex(path); for (auto& ch : ex) ch = (char)std::tolower((unsigned char)ch);
    auto fin = [&](const char* s) { size_t n = std::strlen(s); return ex.size() >= n && ex.compare(ex.size() - n, n, s) == 0; };
    float k = 1.f;
    if (fin(".gltf") || fin(".glb") || fin(".x3d") || fin(".dae")) k = 1000.f;
    else if (fin(".fbx")) {
        double usf = 1.0; float usf_f = 1.f;
        if (sc->mMetaData) {
            if (!sc->mMetaData->Get("UnitScaleFactor", usf) && sc->mMetaData->Get("UnitScaleFactor", usf_f)) usf = usf_f;
        }
        k = (float)(usf * 10.0);
    }
    std::vector<float> v, c; bool col = false;
    for (unsigned m = 0; m < sc->mNumMeshes; ++m) {
        const aiMesh* me = sc->mMeshes[m];
        if (!(me->mPrimitiveTypes & aiPrimitiveType_TRIANGLE)) continue;
        aiColor4D mc(0.93f, 0.80f, 0.33f, 1.f); bool matc = false;
        if (me->mMaterialIndex < sc->mNumMaterials) {
            aiColor4D d; if (sc->mMaterials[me->mMaterialIndex]->Get(AI_MATKEY_COLOR_DIFFUSE, d) == AI_SUCCESS) { mc = d; matc = true; }
        }
        bool vc = me->HasVertexColors(0);
        if (vc || matc) col = true;
        for (unsigned f = 0; f < me->mNumFaces; ++f) {
            const aiFace& fa = me->mFaces[f]; if (fa.mNumIndices != 3) continue;
            // Assimp ramène toute scène en « Y vers le haut » ; 3D-Patch est en « Z vers le haut »
            aiVector3D p[3]; for (int j = 0; j < 3; ++j) { aiVector3D q = me->mVertices[fa.mIndices[j]] * k; p[j] = aiVector3D(q.x, -q.z, q.y); }
            aiVector3D n = (p[1] - p[0]) ^ (p[2] - p[0]); float l = n.Length(); if (l > 0) n /= l;
            for (int j = 0; j < 3; ++j) {
                v.insert(v.end(), { p[j].x, p[j].y, p[j].z, n.x, n.y, n.z });
                aiColor4D cc = vc ? me->mColors[0][fa.mIndices[j]] : mc;
                c.insert(c.end(), { cc.r, cc.g, cc.b });
            }
        }
    }
    if (v.empty()) { err(e, en, "aucun triangle dans le fichier"); return 2; }
    *nv = v.size(); *verts = (float*)std::malloc(v.size() * sizeof(float)); std::memcpy(*verts, v.data(), v.size() * sizeof(float));
    *cols = (float*)std::malloc(c.size() * sizeof(float)); std::memcpy(*cols, c.data(), c.size() * sizeof(float));
    *has_col = col ? 1 : 0;
    return 0;
}

extern "C" void as_free(float* p) { std::free(p); }

extern "C" int as_export(const char* path, const char* format, const float* verts, size_t ntri, const float* cols, char* e, int en) {
    const std::string fmt(format);
    const bool yup = fmt != "3ds";
    const float ech = (fmt == "gltf2" || fmt == "glb2" || fmt == "x3d" || fmt == "collada") ? 0.001f : 1.f; // mm -> m
    aiScene sc;
    sc.mRootNode = new aiNode("3D-Patch");
    sc.mNumMaterials = 1; sc.mMaterials = new aiMaterial*[1]; sc.mMaterials[0] = new aiMaterial();
    aiColor4D jaune(0.93f, 0.80f, 0.33f, 1.f); sc.mMaterials[0]->AddProperty(&jaune, 1, AI_MATKEY_COLOR_DIFFUSE);
    aiString nom("matiere"); sc.mMaterials[0]->AddProperty(&nom, AI_MATKEY_NAME);
    // 3DS : 65535 sommets max par objet -> découpage en plusieurs objets
    const size_t par = 21000; // triangles par objet (3 sommets chacun, < 65535)
    size_t nmesh = (ntri + par - 1) / par; if (nmesh == 0) nmesh = 1;
    sc.mNumMeshes = (unsigned)nmesh; sc.mMeshes = new aiMesh*[nmesh];
    sc.mRootNode->mNumMeshes = (unsigned)nmesh; sc.mRootNode->mMeshes = new unsigned[nmesh];
    for (size_t m = 0; m < nmesh; ++m) {
        size_t t0 = m * par, t1 = std::min(ntri, t0 + par), nt = t1 - t0;
        aiMesh* me = new aiMesh(); me->mName = aiString("objet_" + std::to_string(m + 1));
        me->mMaterialIndex = 0; me->mPrimitiveTypes = aiPrimitiveType_TRIANGLE;
        me->mNumVertices = (unsigned)(nt * 3); me->mVertices = new aiVector3D[nt * 3]; me->mNormals = new aiVector3D[nt * 3];
        if (cols) me->mColors[0] = new aiColor4D[nt * 3];
        me->mNumFaces = (unsigned)nt; me->mFaces = new aiFace[nt];
        for (size_t t = 0; t < nt; ++t) {
            aiFace& f = me->mFaces[t]; f.mNumIndices = 3; f.mIndices = new unsigned[3];
            for (int k = 0; k < 3; ++k) {
                size_t src = ((t0 + t) * 3 + k), dst = t * 3 + k;
                const float* s = verts + src * 6;
                // formats « Y vers le haut » (glTF, FBX, DAE, X3D) : Z-haut -> Y-haut ; le 3DS est nativement Z-haut
                if (yup) { me->mVertices[dst] = aiVector3D(s[0], s[2], -s[1]) * ech; me->mNormals[dst] = aiVector3D(s[3], s[5], -s[4]); }
                else { me->mVertices[dst] = aiVector3D(s[0], s[1], s[2]) * ech; me->mNormals[dst] = aiVector3D(s[3], s[4], s[5]); }
                if (cols) me->mColors[0][dst] = aiColor4D(cols[src * 3], cols[src * 3 + 1], cols[src * 3 + 2], 1.f);
                f.mIndices[k] = (unsigned)dst;
            }
        }
        sc.mMeshes[m] = me; sc.mRootNode->mMeshes[m] = (unsigned)m;
    }
    // unité : millimètre (FBX : facteur 0,1 = 1 unité vaut 0,1 cm ; sans ça les logiciels lisent des cm)
    if (std::string(format) == "fbx") {
        sc.mMetaData = aiMetadata::Alloc(2);
        sc.mMetaData->Set(0, "UnitScaleFactor", (double)0.1);
        sc.mMetaData->Set(1, "OriginalUnitScaleFactor", (double)0.1);
    }
    Assimp::Exporter ex;
    if (ex.Export(&sc, format, path) != AI_SUCCESS) { err(e, en, ex.GetErrorString()); return 1; }
    return 0;
}
