// Passerelle SketchUp (.skp / .skb) via OpenSKP (licence MIT, ../libs/skp-iamahsanmehmood).
// Format des sommets échangés avec Rust : comme assimp_shim — par triangle, 3 × (x y z nx ny nz) en mm,
// Z vers le haut ; couleurs 3 × (r g b) en 0..1 (sRGB).
#include <openskp/openskp.hpp>
#include <openskp/create.hpp>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iterator>
#include <map>
#include <string>
#include <unordered_map>
#include <vector>

static void err(char* e, int en, const std::string& s) {
    if (!e || en <= 0) return;
    std::strncpy(e, s.c_str(), (size_t)en - 1); e[en - 1] = 0;
}
namespace openskp { extern bool front_only_sides; } // correctif local d'OpenSKP (face_groups.cpp)

extern "C" int skp_import(const char* path, float** verts, size_t* nv, float** cols, int* has_col, char* e, int en) {
    try {
        std::ifstream f(path, std::ios::binary);
        if (!f) { err(e, en, "fichier illisible"); return 1; }
        openskp::ByteBuffer buf((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
        openskp::front_only_sides = true; // une seule face par face SketchUp (sinon recto + verso doublés)
        openskp::Scene sc = openskp::build_scene(std::move(buf));
        std::vector<float> v, c; bool col = false;
        for (const auto& p : sc.glb_primitives) {
            float mc[3] = {0.93f, 0.80f, 0.33f};
            if (p.material_index < sc.gltf_materials.size()) {
                const auto& b = sc.gltf_materials[p.material_index].pbr_metallic_roughness.base_color_factor;
                // couleur déjà en sRGB (octets SketchUp / 255) ; gris 136 = face non peinte
                const bool defaut = std::lround(b[0] * 255) == 136 && std::lround(b[1] * 255) == 136 && std::lround(b[2] * 255) == 136;
                if (!defaut) { mc[0] = (float)b[0]; mc[1] = (float)b[1]; mc[2] = (float)b[2]; col = true; }
            }
            const auto& P = p.positions;
            for (size_t t = 0; t + 2 < p.indices.size(); t += 3) {
                float q[3][3];
                bool ok = true;
                for (int j = 0; j < 3; ++j) {
                    size_t i = p.indices[t + j];
                    if (i * 3 + 2 >= P.size()) { ok = false; break; }
                    // scène glTF : mètres, Y vers le haut -> mm, Z vers le haut
                    q[j][0] = P[i * 3] * 1000.f; q[j][1] = -P[i * 3 + 2] * 1000.f; q[j][2] = P[i * 3 + 1] * 1000.f;
                }
                if (!ok) continue;
                float ux = q[1][0] - q[0][0], uy = q[1][1] - q[0][1], uz = q[1][2] - q[0][2];
                float wx = q[2][0] - q[0][0], wy = q[2][1] - q[0][1], wz = q[2][2] - q[0][2];
                float nx = uy * wz - uz * wy, ny = uz * wx - ux * wz, nz = ux * wy - uy * wx;
                float l = std::sqrt(nx * nx + ny * ny + nz * nz); if (l > 0) { nx /= l; ny /= l; nz /= l; }
                for (int j = 0; j < 3; ++j) {
                    v.insert(v.end(), {q[j][0], q[j][1], q[j][2], nx, ny, nz});
                    c.insert(c.end(), {mc[0], mc[1], mc[2]});
                }
            }
        }
        if (v.empty()) { err(e, en, "aucune face dans le fichier SketchUp"); return 2; }
        *nv = v.size();
        *verts = (float*)std::malloc(v.size() * sizeof(float)); std::memcpy(*verts, v.data(), v.size() * sizeof(float));
        *cols = (float*)std::malloc(c.size() * sizeof(float)); std::memcpy(*cols, c.data(), c.size() * sizeof(float));
        *has_col = col ? 1 : 0;
        return 0;
    } catch (const std::exception& x) {
        err(e, en, std::string("SketchUp : ") + x.what()); return 3;
    } catch (...) {
        err(e, en, "SketchUp : erreur inconnue"); return 3;
    }
}

extern "C" void skp_free(float* p) { std::free(p); }

// Export : triangles -> faces SketchUp (pouces, Z vers le haut), une matière par couleur distincte.
// Les arêtes entre triangles quasi coplanaires sont adoucies (aspect « surface » dans SketchUp).
extern "C" int skp_export(const char* path, const float* verts, size_t ntri, const float* cols, char* e, int en) {
    try {
        const double k = 1.0 / 25.4; // mm -> pouces
        // sommets fusionnés (mêmes coordonnées exactes => arêtes partagées par OpenSKP)
        auto key = [&](size_t vi) { const float* s = verts + vi * 6; return std::array<float, 3>{s[0], s[1], s[2]}; };
        std::map<std::array<float, 3>, uint32_t> ids;
        std::vector<uint32_t> tri(ntri * 3);
        for (size_t i = 0; i < ntri * 3; ++i) tri[i] = ids.emplace(key(i), (uint32_t)ids.size()).first->second;
        // normales de faces et arêtes -> triangles voisins
        std::vector<std::array<float, 3>> fn(ntri);
        for (size_t t = 0; t < ntri; ++t) {
            const float *a = verts + t * 18, *b = a + 6, *c = a + 12;
            float ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2], wx = c[0] - a[0], wy = c[1] - a[1], wz = c[2] - a[2];
            float nx = uy * wz - uz * wy, ny = uz * wx - ux * wz, nz = ux * wy - uy * wx;
            float l = std::sqrt(nx * nx + ny * ny + nz * nz); if (l > 0) { nx /= l; ny /= l; nz /= l; }
            fn[t] = {nx, ny, nz};
        }
        std::unordered_map<uint64_t, std::vector<uint32_t>> edges;
        auto ek = [](uint32_t a, uint32_t b) { if (a > b) std::swap(a, b); return ((uint64_t)a << 32) | b; };
        for (size_t t = 0; t < ntri; ++t)
            for (int j = 0; j < 3; ++j) edges[ek(tri[t * 3 + j], tri[t * 3 + (j + 1) % 3])].push_back((uint32_t)t);
        const float cos_lim = std::cos(20.0f * 3.14159265f / 180.0f);
        auto lisse = [&](size_t t) {
            for (int j = 0; j < 3; ++j) {
                const auto& v = edges[ek(tri[t * 3 + j], tri[t * 3 + (j + 1) % 3])];
                if (v.size() != 2) return false;
                uint32_t o = v[0] == t ? v[1] : v[0];
                float d = fn[t][0] * fn[o][0] + fn[t][1] * fn[o][1] + fn[t][2] * fn[o][2];
                if (d < cos_lim) return false;
            }
            return true;
        };

        auto b = openskp::create();
        // matières : une par couleur (quantifiée à 8 bits) ; toutes déclarées avant la 1re face
        std::vector<int> mat(ntri, -1);
        if (cols) {
            std::map<uint32_t, int> parcoul;
            for (size_t t = 0; t < ntri; ++t) {
                const float* c = cols + t * 9;
                auto q = [](float x) { return (uint32_t)std::lround(std::clamp(x, 0.f, 1.f) * 255.f); };
                uint32_t r = q(c[0]), g = q(c[1]), bl = q(c[2]);
                uint32_t cle = (r << 16) | (g << 8) | bl;
                auto it = parcoul.find(cle);
                if (it == parcoul.end()) {
                    char nom[32]; std::snprintf(nom, sizeof nom, "Couleur_%02X%02X%02X", r, g, bl);
                    int m = b->add_material(nom, openskp::Color3{(uint8_t)r, (uint8_t)g, (uint8_t)bl});
                    it = parcoul.emplace(cle, m).first;
                }
                mat[t] = it->second;
            }
        }
        for (size_t t = 0; t < ntri; ++t) {
            std::vector<openskp::Point3> pts(3);
            for (int j = 0; j < 3; ++j) { const float* s = verts + (t * 3 + j) * 6; pts[j] = {s[0] * k, s[1] * k, s[2] * k}; }
            if (tri[t * 3] == tri[t * 3 + 1] || tri[t * 3 + 1] == tri[t * 3 + 2] || tri[t * 3] == tri[t * 3 + 2]) continue; // dégénéré
            openskp::FaceOptions o;
            if (mat[t] >= 0) { o.material = mat[t]; o.back_material = mat[t]; }
            if (lisse(t)) { o.soft_edges = true; o.smooth_edges = true; }
            b->add_face(pts, o);
        }
        b->save(path);
        return 0;
    } catch (const std::exception& x) {
        err(e, en, std::string("SketchUp : ") + x.what()); return 1;
    } catch (...) {
        err(e, en, "SketchUp : erreur inconnue"); return 1;
    }
}
